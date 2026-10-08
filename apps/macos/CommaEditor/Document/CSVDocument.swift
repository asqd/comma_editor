import AppKit
import UniformTypeIdentifiers

/// A CSV file open in the app. The data and the edit history live in the Rust core;
/// each core edit is mirrored by one step in the document's undo manager.
final class CSVDocument: NSDocument {
    enum Change {
        /// One cell of a visible row changed.
        case cell(visibleRow: Int, column: Int)
        /// The header row changed.
        case header
        /// Rows, columns, the view or the header setting changed.
        case structure
        /// More rows of a large file were indexed, or background sorting started or stopped.
        case progress
    }

    /// Tables from this many rows on are sorted, filtered and changed off the main thread.
    private static let backgroundRows: UInt64 = 200_000
    /// Bytes indexed per step of a large file.
    private static let indexingStep: UInt64 = 64 << 20

    /// Share of a large file indexed so far, while it is being indexed.
    private(set) var indexingFraction: Double?
    var isIndexing: Bool { indexingFraction != nil }
    /// Sorting or filtering running off the main thread.
    private var viewJob: ViewJob?
    var isUpdatingView: Bool { viewJob != nil }
    /// A change to many rows running off the main thread, and what the status bar calls it.
    private var editJob: EditJob?
    private(set) var editStatus: String?
    /// Share of the running change done, while one runs.
    var editFraction: Double? { editJob?.fraction() }
    /// The file cannot be changed now: it is being read, or another change runs.
    var isBusy: Bool { isIndexing || editJob != nil }

    static let didChangeNotification = Notification.Name("CSVDocumentDidChange")
    static let changeKey = "change"

    private(set) var core: CoreDocument
    /// Encoding and delimiter chosen with File ▸ Reopen; `nil` detects them.
    private(set) var chosenEncoding: TextEncoding?
    private(set) var chosenDelimiter: String?

    /// Encodings a file can be reopened in, in menu order.
    static let encodings: [TextEncoding] = [.utf8, .utf16Le, .utf16Be, .windows1251, .koi8R, .windows1252]
    /// Delimiters a file can be reopened with, in menu order.
    static let delimiters = [",", ";", "\t", "|"]

    /// The format controls of the save panel being shown.
    private var formatAccessory: SaveFormatAccessoryView?
    /// Format of the copy being exported; the document keeps its own.
    private var exportFormat: SaveFormat?
    private var exportsForExcel = false

    override init() {
        core = CoreDocument.newEmpty()
        super.init()
    }

    override class var autosavesInPlace: Bool { true }

    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { true }

    override func makeWindowControllers() {
        addWindowController(DocumentWindowController())
    }

    // MARK: Reading and writing

    override func read(from url: URL, ofType typeName: String) throws {
        do {
            core = try CoreDocument.openWith(path: url.path, encoding: chosenEncoding, delimiter: chosenDelimiter)
        } catch {
            throw Self.userFacingError(error)
        }
        startIndexing()
    }

    /// A large file opens with its first part; the rest is indexed off the main thread. Until
    /// that ends the document cannot be edited or saved.
    private func startIndexing() {
        let core = core
        guard core.isIndexing() else {
            indexingFraction = nil
            return
        }
        indexingFraction = 0
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var done = false
            while !done {
                let progress = core.continueIndexing(bytes: Self.indexingStep)
                done = progress.done
                DispatchQueue.main.async {
                    // A reopened document has another core; the old one finishes on its own.
                    guard let self, self.core === core else { return }
                    self.indexingFraction = done
                        ? nil
                        : Double(progress.indexedBytes) / Double(max(progress.totalBytes, 1))
                    self.post(done ? .structure : .progress)
                }
            }
        }
    }

    /// Reads the file again with an encoding and delimiter (`nil` detects them). Unsaved
    /// changes and the undo history are dropped; callers ask first.
    func reopen(encoding: TextEncoding?, delimiter: String?) {
        guard let url = fileURL else { return }
        let previous = (chosenEncoding, chosenDelimiter)
        chosenEncoding = encoding
        chosenDelimiter = delimiter
        do {
            core = try CoreDocument.openWith(path: url.path, encoding: encoding, delimiter: delimiter)
        } catch {
            (chosenEncoding, chosenDelimiter) = previous
            presentEditError(error)
            return
        }
        undoManager?.removeAllActions()
        updateChangeCount(.changeCleared)
        viewJob?.cancel()
        viewJob = nil
        editJob?.cancel()
        editJob = nil
        startIndexing()
        post(.structure)
    }

    // MARK: File ▸ Reopen

    @objc func reopenWithEncoding(_ sender: Any?) {
        guard let item = sender as? NSMenuItem else { return }
        let encoding = item.tag < 0 ? nil : Self.encodings[item.tag]
        confirmReopen { $0.reopen(encoding: encoding, delimiter: $0.chosenDelimiter) }
    }

    @objc func reopenWithDelimiter(_ sender: Any?) {
        guard let item = sender as? NSMenuItem else { return }
        let delimiter = item.tag < 0 ? nil : Self.delimiters[item.tag]
        confirmReopen { $0.reopen(encoding: $0.chosenEncoding, delimiter: delimiter) }
    }

    /// Reopening drops unsaved changes, so it asks first when there are some.
    func confirmReopen(_ reopen: @escaping (CSVDocument) -> Void) {
        guard isDocumentEdited, let window = windowForSheet else {
            reopen(self)
            return
        }
        let alert = NSAlert()
        alert.messageText = L("Reopen the file and lose unsaved changes?")
        alert.informativeText = L("The file is read again from disk; changes made since the last save are lost.")
        alert.addButton(withTitle: L("Reopen"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            reopen(self)
        }
    }

    override func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(reopenWithEncoding(_:)):
            let current = chosenEncoding.map { Self.encodings.firstIndex(of: $0) ?? -2 } ?? -1
            menuItem.state = menuItem.tag == current ? .on : .off
            return fileURL != nil
        case #selector(reopenWithDelimiter(_:)):
            let current = chosenDelimiter.map { Self.delimiters.firstIndex(of: $0) ?? -2 } ?? -1
            menuItem.state = menuItem.tag == current ? .on : .off
            return fileURL != nil
        default:
            return super.validateMenuItem(menuItem)
        }
    }

    static func encodingName(_ encoding: TextEncoding) -> String {
        switch encoding {
        case .utf8: "UTF-8"
        case .utf16Le: "UTF-16LE"
        case .utf16Be: "UTF-16BE"
        case .windows1251: "Windows-1251"
        case .koi8R: "KOI8-R"
        case .windows1252: "Windows-1252"
        }
    }

    static func delimiterName(_ delimiter: String) -> String {
        switch delimiter {
        case ",": L("Comma ( , )")
        case ";": L("Semicolon ( ; )")
        case "\t": L("Tab ( ⇥ )")
        case "|": L("Vertical Bar ( | )")
        default: delimiter
        }
    }

    override func revert(toContentsOf url: URL, ofType typeName: String) throws {
        try super.revert(toContentsOf: url, ofType: typeName)
        undoManager?.removeAllActions()
        post(.structure)
    }

    /// In its own format, unchanged records are copied byte for byte, so saving never
    /// reformats the file.
    override func write(to url: URL, ofType typeName: String) throws {
        do {
            if let exportFormat {
                try core.saveWith(path: url.path, format: exportFormat)
            } else {
                try core.save(path: url.path)
            }
        } catch {
            throw Self.userFacingError(error)
        }
    }

    // MARK: Save panel

    /// The panel shows our format controls instead of NSDocument's file type menu.
    override var shouldRunSavePanelWithAccessoryView: Bool { false }

    override func prepareSavePanel(_ savePanel: NSSavePanel) -> Bool {
        let accessory = SaveFormatAccessoryView(format: core.saveFormat())
        accessory.onChange = { [weak savePanel] format in
            savePanel?.allowedContentTypes = [SaveFormatAccessoryView.contentType(for: format)]
        }
        if exportsForExcel {
            exportsForExcel = false
            accessory.applyExcelPreset()
        }
        savePanel.accessoryView = accessory
        savePanel.allowedContentTypes = [SaveFormatAccessoryView.contentType(for: accessory.format)]
        formatAccessory = accessory
        return true
    }

    override var fileTypeFromLastRunSavePanel: String? {
        formatAccessory.map { SaveFormatAccessoryView.contentType(for: $0.format).identifier }
            ?? super.fileTypeFromLastRunSavePanel
    }

    /// "Save As" switches the document to the chosen format; "Export" writes a copy in it.
    override func save(
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        var previousFormat: SaveFormat?
        if let accessory = formatAccessory {
            formatAccessory = nil
            switch saveOperation {
            case .saveAsOperation:
                previousFormat = core.saveFormat()
                do {
                    try core.setSaveFormat(format: accessory.format)
                } catch {
                    completionHandler(Self.userFacingError(error))
                    return
                }
            case .saveToOperation:
                exportFormat = accessory.format
            default:
                break
            }
        }
        super.save(to: url, ofType: typeName, for: saveOperation) { [weak self] error in
            guard let self else { return completionHandler(error) }
            self.exportFormat = nil
            if error != nil, let previousFormat {
                try? self.core.setSaveFormat(format: previousFormat)
            }
            self.post(.structure)
            completionHandler(error)
        }
    }

    /// File ▸ Export for Excel: the export panel with the Excel preset chosen.
    @objc func exportForExcel(_ sender: Any?) {
        exportsForExcel = true
        saveTo(sender)
    }

    // MARK: Editing
    //
    // Rows are visible rows: data rows as the current sort and filters show them.

    @discardableResult
    func setCell(visibleRow: Int, column: Int, value: String) -> Bool {
        guard let row = core.visibleRecord(visibleRow: UInt64(visibleRow)) else { return false }
        return perform(L("Cell Edit"), .cell(visibleRow: visibleRow, column: column)) {
            try $0.setCell(row: row, column: UInt64(column), value: value)
        }
    }

    @discardableResult
    func renameColumn(_ column: Int, to name: String) -> Bool {
        perform(L("Column Rename"), .header) {
            try $0.setCell(row: 0, column: UInt64(column), value: name)
        }
    }

    /// Inserts empty rows before a visible row (or after the last one).
    @discardableResult
    func insertRows(beforeVisibleRow visibleRow: Int, count: Int) -> Bool {
        perform(L("Row Insertion"), .structure) {
            let row = $0.insertionRow(visibleRow: UInt64(visibleRow))
            try $0.insertRows(at: row, count: UInt64(count))
            return true
        }
    }

    @discardableResult
    func deleteRows(visibleRows: [Int]) -> Bool {
        perform(L("Row Deletion"), .structure) { core in
            let rows = visibleRows.compactMap { core.visibleRecord(visibleRow: UInt64($0)) }
            try core.deleteRows(rows: rows)
            return !rows.isEmpty
        }
    }

    /// Moves visible rows to stand before a visible row (or after the last one).
    @discardableResult
    func moveRows(visibleRows: [Int], beforeVisibleRow target: Int) -> Bool {
        perform(L("Row Move"), .structure) { core in
            let rows = visibleRows.compactMap { core.visibleRecord(visibleRow: UInt64($0)) }
            return try core.moveRows(rows: rows, to: core.insertionRow(visibleRow: UInt64(target)))
        }
    }

    @discardableResult
    func moveColumn(_ column: Int, to target: Int) -> Bool {
        perform(L("Column Move"), .structure) {
            try $0.moveColumn(from: UInt64(column), to: UInt64(target))
        }
    }

    @discardableResult
    func insertColumn(at column: Int) -> Bool {
        perform(L("Column Insertion"), .structure) {
            try $0.insertColumn(at: UInt64(column))
            return true
        }
    }

    @discardableResult
    func deleteColumn(at column: Int) -> Bool {
        perform(L("Column Deletion"), .structure) {
            try $0.deleteColumn(at: UInt64(column))
            return true
        }
    }

    /// Reorders the rows of the file by the current sort.
    @discardableResult
    func applySort() -> Bool {
        perform(L("Sorting"), .structure) { try $0.applySort() }
    }

    @discardableResult
    func replaceInCell(visibleRow: Int, column: Int, query: SearchQuery, replacement: String) -> Bool {
        perform(L("Replacement"), .cell(visibleRow: visibleRow, column: column)) {
            try $0.replaceInCell(
                visibleRow: UInt64(visibleRow),
                column: UInt64(column),
                query: query,
                replacement: replacement
            )
        }
    }

    /// Replaces every match in the visible rows; `done` gets the number of changed cells.
    func replaceAll(query: SearchQuery, replacement: String, done: @escaping (Int) -> Void) {
        edit(L("Replacement"), .replaceAll(query: query, replacement: replacement)) { done(Int($0.count)) }
    }

    /// Sets every cell of a range of visible cells; an empty value clears them.
    @discardableResult
    func fillRange(_ range: CellRange, value: String, actionName: String) -> Bool {
        perform(actionName, .structure) { try $0.fillRange(range: range, value: value) > 0 }
    }

    /// Copies the top row of the range into the rows below it.
    @discardableResult
    func fillDown(_ range: CellRange) -> Bool {
        perform(L("Filling Down"), .structure) { try $0.fillDown(range: range) > 0 }
    }

    /// Writes cells from (`row`, `column`), adding rows and columns as needed; returns the range written.
    func paste(_ grid: [[String]], atRow row: Int, column: Int) -> CellRange? {
        var written: CellRange?
        perform(L("Pasting"), .structure) {
            written = try $0.paste(top: UInt64(row), left: UInt64(column), grid: grid)
            return written != nil
        }
        return written
    }

    // MARK: Column operations

    func transform(_ range: CellRange, _ transform: TextTransform, actionName: String) {
        edit(actionName, .transform(range: range, transform: transform))
    }

    func fillEmpty(_ range: CellRange, fromAbove: Bool, value: String) {
        edit(L("Filling Empty Cells"), .fillEmpty(range: range, fromAbove: fromAbove, value: value))
    }

    /// `done` gets the number of columns added.
    func splitColumn(_ column: Int, separator: String, regex: Bool, done: @escaping (Int) -> Void) {
        edit(L("Column Split"), .splitColumn(column: UInt64(column), separator: separator, regex: regex)) {
            done(Int($0.count))
        }
    }

    func mergeColumns(_ columns: [Int], separator: String) {
        edit(L("Column Merge"), .mergeColumns(columns: columns.map { UInt64($0) }, separator: separator))
    }

    /// `done` gets the number of rows removed.
    func removeDuplicates(columns: [Int], normalize: Bool, done: @escaping (Int) -> Void) {
        edit(L("Duplicate Removal"), .removeDuplicates(columns: columns.map { UInt64($0) }, normalize: normalize)) {
            done(Int($0.count))
        }
    }

    // MARK: Formulas

    /// Inserts a column computed by the formula before column `at`.
    func addFormulaColumn(_ formula: String, name: String, at column: Int, done: @escaping (FormulaReport) -> Void) {
        edit(L("Computed Column"), .addFormulaColumn(formula: formula, name: name, at: UInt64(column))) {
            $0.report.map(done)
        }
    }

    /// Writes the formula's value into a column of every visible row.
    func fillColumn(_ column: Int, formula: String, done: @escaping (FormulaReport) -> Void) {
        edit(L("Computed Column"), .fillWithFormula(formula: formula, column: UInt64(column))) {
            $0.report.map(done)
        }
    }

    /// Fixes text read in the wrong encoding in a range, or in the whole file; `done` gets the
    /// number of fixed cells.
    func fixGarbledText(_ range: CellRange?, done: @escaping (Int) -> Void = { _ in }) {
        edit(L("Garbled Text Fix"), .fixGarbled(range: range)) { done(Int($0.count)) }
    }

    // MARK: Changes to many rows

    /// Makes a change to many rows as one undoable step. Large tables are changed off the main
    /// thread, with progress and Cancel in the status bar; the table can be read meanwhile, not
    /// changed. `done` runs once the change is made, unless it failed or was cancelled.
    func edit(_ actionName: String, _ request: EditRequest, done: @escaping (EditResult) -> Void = { _ in }) {
        guard editJob == nil else {
            NSSound.beep()
            return
        }
        guard core.rowCount() >= Self.backgroundRows else {
            do {
                finishEdit(actionName, try core.applyEdit(request: request), done)
            } catch {
                presentEditError(error)
            }
            return
        }
        let job: EditJob
        do {
            job = try core.beginEdit(request: request)
        } catch {
            presentEditError(error)
            return
        }
        editJob = job
        editStatus = Self.status(of: request)
        post(.progress)
        let progress = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in self?.post(.progress) }
        RunLoop.main.add(progress, forMode: .common)
        let core = core
        let started = Date()
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            _ = job.run()
            DispatchQueue.main.async {
                progress.invalidate()
                // Always finished, so the core takes changes again; a reopened document has
                // another core, and the old one is let go.
                let applying = Date()
                let result = Result { try core.finishEdit(job: job) }
                #if DEBUG
                NSLog("COMMA edit \(actionName): %.2f s in the background, %.3f s applying",
                      applying.timeIntervalSince(started), Date().timeIntervalSince(applying))
                #endif
                guard let self, self.core === core else { return }
                self.editJob = nil
                self.editStatus = nil
                switch result {
                case .success(let result?):
                    self.finishEdit(actionName, result, done)
                case .success(nil):
                    self.post(.progress)
                case .failure(let error):
                    self.post(.progress)
                    self.presentEditError(error)
                }
            }
        }
    }

    private static func status(of request: EditRequest) -> String {
        switch request {
        case .replaceAll: L("Replacing…")
        case .transform: L("Changing text…")
        case .fillEmpty: L("Filling empty cells…")
        case .fixGarbled: L("Fixing garbled text…")
        case .addFormulaColumn, .fillWithFormula: L("Computing the column…")
        case .splitColumn: L("Splitting the column…")
        case .mergeColumns: L("Merging columns…")
        case .removeDuplicates: L("Removing duplicates…")
        }
    }

    /// Stops the change running off the main thread; the file stays as it was.
    func cancelEdit() {
        editJob?.cancel()
    }

    private func finishEdit(_ actionName: String, _ result: EditResult, _ done: (EditResult) -> Void) {
        if result.changed {
            registerUndo()
            undoManager?.setActionName(actionName)
        }
        post(.structure)
        done(result)
    }

    // MARK: Repair

    /// Fixes the broken row `row` of the file (the header row counted).
    @discardableResult
    func applyFix(row: Int, fix: RepairFix) -> Bool {
        perform(L("Row Repair"), .structure) { try $0.applyFix(row: UInt64(row), fix: fix) }
    }

    /// Applies the recommended fix to every broken row of one kind; returns the rows fixed.
    @discardableResult
    func fixAll(_ kind: ProblemKind) -> Int {
        var fixed: UInt64 = 0
        perform(L("Row Repair"), .structure) {
            fixed = $0.fixAll(kind: kind)
            return fixed > 0
        }
        return Int(fixed)
    }

    /// Sorting and filtering change what is shown, not the file, so they are not undoable.
    /// Throws on an invalid regular expression; callers show the error where it was typed.
    /// Large tables are sorted and filtered off the main thread: `shown` runs once the new view
    /// is in place, and a newer request cancels an older one.
    func setView(_ spec: ViewSpec, shown: @escaping () -> Void = {}) throws {
        viewJob?.cancel()
        viewJob = nil
        if core.rowCount() < Self.backgroundRows {
            try core.setView(spec: spec)
            post(.structure)
            shown()
            return
        }
        let job = try core.beginView(spec: spec)
        viewJob = job
        post(.progress)
        let core = core
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let finished = job.run()
            DispatchQueue.main.async {
                guard let self, self.viewJob === job else { return }
                self.viewJob = nil
                if finished, core.finishView(job: job) {
                    self.post(.structure)
                    shown()
                } else if finished {
                    // The rows changed while sorting: sort again.
                    try? self.setView(spec, shown: shown)
                } else {
                    self.post(.progress)
                }
            }
        }
    }

    /// Stops background sorting or filtering; the previous view stays.
    func cancelViewUpdate() {
        viewJob?.cancel()
        viewJob = nil
        post(.progress)
    }

    /// Display setting only: the file does not change.
    func setHasHeader(_ hasHeader: Bool) {
        core.setHasHeader(hasHeader: hasHeader)
        post(.structure)
    }

    /// Runs one core edit. If it changed something, registers an undo step and notifies the UI.
    @discardableResult
    private func perform(_ actionName: String, _ change: Change, _ edit: (CoreDocument) throws -> Bool) -> Bool {
        do {
            guard try edit(core) else { return false }
        } catch {
            presentEditError(error)
            return false
        }
        registerUndo()
        undoManager?.setActionName(actionName)
        post(change)
        return true
    }

    // The core keeps the history; the undo manager only drives it, so NSDocument tracks
    // unsaved changes and the Edit menu shows the right titles.
    private func registerUndo() {
        undoManager?.registerUndo(withTarget: self) { document in
            _ = document.core.undo()
            document.registerRedo()
            document.post(.structure)
        }
    }

    private func registerRedo() {
        undoManager?.registerUndo(withTarget: self) { document in
            _ = document.core.redo()
            document.registerUndo()
            document.post(.structure)
        }
    }

    private func post(_ change: Change) {
        NotificationCenter.default.post(
            name: Self.didChangeNotification,
            object: self,
            userInfo: [Self.changeKey: change]
        )
    }

    // MARK: Errors

    func presentEditError(_ error: Error) {
        let error = Self.userFacingError(error)
        if let window = windowForSheet {
            presentError(error, modalFor: window, delegate: nil, didPresent: nil, contextInfo: nil)
        } else {
            presentError(error)
        }
    }

    static func userFacingError(_ error: Error) -> NSError {
        guard let error = error as? CoreError else { return error as NSError }
        var userInfo: [String: Any] = [:]
        switch error {
        case .Io(let message), .OutOfRange(let message):
            userInfo[NSLocalizedDescriptionKey] = message
        case .Unmappable(let row, let column, let encoding):
            userInfo[NSLocalizedDescriptionKey] = L(
                "The value in row %lld, column %lld cannot be saved in %@.",
                Int64(row) + 1, Int64(column) + 1, encoding
            )
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = L(
                "Replace the characters that this encoding cannot represent."
            )
        case .LastColumn:
            userInfo[NSLocalizedDescriptionKey] = L("The last column cannot be deleted.")
        case .Busy:
            userInfo[NSLocalizedDescriptionKey] = L("The file is still being read.")
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = L("Changes and saving are possible once the whole file is read; the progress is in the status bar.")
        case .Editing:
            userInfo[NSLocalizedDescriptionKey] = L("Another change is being made.")
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = L("Wait until it ends or cancel it in the status bar.")
        case .InvalidDelimiter(let message), .Export(let message):
            userInfo[NSLocalizedDescriptionKey] = message
        case .Formula(let problem):
            userInfo[NSLocalizedDescriptionKey] = FormulaHelp.describe(problem)
        case .InvalidPattern(let message):
            userInfo[NSLocalizedDescriptionKey] = L("The regular expression is not valid.")
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = message
        }
        return NSError(domain: "app.commaeditor", code: 1, userInfo: userInfo)
    }
}
