import AppKit

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
    }

    static let didChangeNotification = Notification.Name("CSVDocumentDidChange")
    static let changeKey = "change"

    private(set) var core: CoreDocument

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
            core = try CoreDocument.open(path: url.path)
        } catch {
            throw Self.userFacingError(error)
        }
    }

    override func revert(toContentsOf url: URL, ofType typeName: String) throws {
        try super.revert(toContentsOf: url, ofType: typeName)
        undoManager?.removeAllActions()
        post(.structure)
    }

    /// Unchanged records are copied byte for byte, so saving never reformats the file.
    override func write(to url: URL, ofType typeName: String) throws {
        do {
            try core.save(path: url.path)
        } catch {
            throw Self.userFacingError(error)
        }
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

    /// Replaces every match in the visible rows; returns the number of changed cells.
    func replaceAll(query: SearchQuery, replacement: String) -> Int {
        var count: UInt64 = 0
        perform(L("Replacement"), .structure) {
            count = try $0.replaceAll(query: query, replacement: replacement)
            return count > 0
        }
        return Int(count)
    }

    /// Sorting and filtering change what is shown, not the file, so they are not undoable.
    /// Throws on an invalid regular expression; callers show the error where it was typed.
    func setView(_ spec: ViewSpec) throws {
        try core.setView(spec: spec)
        post(.structure)
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

    private func presentEditError(_ error: Error) {
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
        case .InvalidPattern(let message):
            userInfo[NSLocalizedDescriptionKey] = L("The regular expression is not valid.")
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = message
        }
        return NSError(domain: "app.commaeditor", code: 1, userInfo: userInfo)
    }
}
