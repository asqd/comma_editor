import AppKit
import UniformTypeIdentifiers

/// An Excel workbook open in the app. Each sheet is a table of its own, read from the file when
/// first shown; one sheet at a time is the document's table. Saving writes the edits of every
/// sheet into the file opened, changing only what changed. Excel 97–2003 workbooks open for
/// viewing only: they can be sorted, filtered, searched and exported, not changed or saved.
final class WorkbookDocument: CSVDocument {
    private(set) var workbook: CoreWorkbook?
    /// Sheets in the order of their tabs.
    private(set) var sheets: [SheetInfo] = []
    /// The sheet shown, as an index into `sheets`.
    private(set) var sheetIndex = 0
    /// Counts reads of the file, so the window drops what it kept about sheets of an older one.
    private(set) var generation = 0
    private var loaded: [Int: LoadedSheet] = [:]
    /// A sheet being read off the main thread.
    private var loading: (index: Int, job: SheetJob)?

    /// An Excel 97–2003 workbook (`.xls`), whose formulas are not shown.
    var isLegacyFormat: Bool { workbook?.isLegacyFormat() ?? false }

    /// The layout of the shown sheet: column widths, hidden columns, frozen panes.
    var layout: SheetLayout? { loaded[sheetIndex]?.layout }

    /// After the last change: formulas that read themselves, and ones CommaEditor does not
    /// compute, which Excel computes when it opens the file.
    private(set) var circularFormulas = 0
    private(set) var formulasNotComputed = 0

    /// The sheet being read, if one is.
    var loadingSheetIndex: Int? { loading?.index }

    /// CSV mode: the first row of every sheet names the columns, as in a delimited file. Off,
    /// sheets show as Excel shows them: columns lettered A, B, C, every row a row of cells.
    private(set) var csvMode = false

    /// The name and the share read of a sheet being read.
    var loadingSheet: (name: String, fraction: Double)? {
        loading.map { (sheets[$0.index].name, $0.job.fraction()) }
    }

    override var isReadOnly: Bool { isLegacyFormat }

    /// Save As runs: the panel offers the workbook's own format rather than CSV.
    private var savingAsWorkbook = false
    /// The last save panel was for the workbook itself, not for a CSV copy.
    private var panelSavedWorkbook = false

    // Nothing is ever written to the workbook itself.
    override class var autosavesInPlace: Bool { false }

    override func read(from url: URL, ofType typeName: String) throws {
        do {
            let workbook = try CoreWorkbook.open(path: url.path)
            let sheets = workbook.sheets()
            guard sheets.contains(where: \.isTable) else {
                throw NSError(domain: "app.commaeditor", code: 2, userInfo: [
                    NSLocalizedDescriptionKey: L("The workbook has no sheets with cells."),
                ])
            }
            // Read again, as on Revert: the sheet shown stays shown; first, the one Excel opens at.
            let shown = self.sheets.indices.contains(sheetIndex) && self.workbook != nil ? self.sheets[sheetIndex].name : nil
            let index = shown.flatMap { name in sheets.firstIndex { $0.isTable && $0.name == name } } ?? Int(workbook.activeSheet())
            guard let sheet = try workbook.readSheet(index: UInt64(index), locale: Self.displayLocale, job: SheetJob()) else {
                throw CocoaError(.userCancelled)
            }
            // A file written by a program asks to be computed on opening, as Excel does: its
            // sheets with formulas are read now, so formulas reading other sheets get results.
            var read = [index: sheet]
            for other in try workbook.sheetsComputedOnOpen().map({ Int($0) }) where read[other] == nil {
                if let extra = try workbook.readSheet(index: UInt64(other), locale: Self.displayLocale, job: SheetJob()) {
                    read[other] = extra
                }
            }
            loading?.job.cancel()
            loading = nil
            self.workbook = workbook
            self.sheets = sheets
            loaded = read.mapValues(adopt)
            sheetIndex = index
            generation += 1
            // A formula that cannot be computed keeps its stored result: opening goes on.
            _ = try? computeFormulas()
            replaceCore(sheet.document)
        } catch let error as CoreError {
            throw Self.userFacingError(error)
        }
    }

    // MARK: Sheets

    /// Shows a sheet, reading it first if it was not shown before; a large sheet is read off
    /// the main thread with its progress in the status bar.
    func showSheet(_ index: Int) {
        guard sheets.indices.contains(index), sheets[index].isTable, let workbook else { return }
        loading?.job.cancel()
        loading = nil
        if let sheet = loaded[index] {
            show(sheet, at: index)
            return
        }
        let job = SheetJob()
        loading = (index, job)
        post(.progress)
        let progress = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in self?.post(.progress) }
        RunLoop.main.add(progress, forMode: .common)
        let locale = Self.displayLocale
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result { try workbook.readSheet(index: UInt64(index), locale: locale, job: job) }
            DispatchQueue.main.async {
                progress.invalidate()
                guard let self, self.loading?.job === job else { return }
                self.loading = nil
                switch result {
                case .success(let sheet?):
                    self.loaded[index] = self.adopt(sheet)
                    self.show(sheet, at: index)
                case .success(nil):
                    self.post(.progress)
                case .failure(let error):
                    self.post(.progress)
                    self.presentEditError(error)
                }
            }
        }
    }

    /// Stops reading a sheet; the shown one stays.
    func cancelSheetLoading() {
        loading?.job.cancel()
        loading = nil
        post(.progress)
    }

    private func show(_ sheet: LoadedSheet, at index: Int) {
        sheetIndex = index
        // A sheet read just now may have formulas that read cells edited on other sheets.
        _ = tableDidChange()
        replaceCore(sheet.document, keepsUndo: true)
    }

    // MARK: Rows and columns

    /// Whether inserting and deleting rows here goes through the workbook, which moves the
    /// references to them: their own does not when something refers to them.
    private var rowsMoveReferences: Bool {
        let info = core.info()
        return info.sheetRowsFixed && !info.sheetStructureLocked
    }

    @discardableResult
    override func insertRows(beforeVisibleRow visibleRow: Int, count: Int) -> Bool {
        let row = Int(core.insertionRow(visibleRow: UInt64(visibleRow)))
        // Rows added at the end move nothing.
        guard rowsMoveReferences, row < Int(core.rowCount()) else {
            return super.insertRows(beforeVisibleRow: visibleRow, count: count)
        }
        return restructure(L("Row Insertion"), .rows, insert: true, runs: [(row, count)])
    }

    @discardableResult
    override func deleteRows(visibleRows: [Int]) -> Bool {
        guard rowsMoveReferences else { return super.deleteRows(visibleRows: visibleRows) }
        let rows = visibleRows.compactMap { core.visibleRecord(visibleRow: UInt64($0)).map { Int($0) } }
        return restructure(L("Row Deletion"), .rows, insert: false, runs: Self.runs(of: rows))
    }

    @discardableResult
    override func insertColumn(at column: Int) -> Bool {
        guard column < Int(core.columnCount()), !core.info().sheetStructureLocked else {
            return super.insertColumn(at: column)
        }
        return restructure(L("Column Insertion"), .columns, insert: true, runs: [(column, 1)])
    }

    @discardableResult
    override func deleteColumn(at column: Int) -> Bool {
        guard !core.info().sheetStructureLocked else { return super.deleteColumn(at: column) }
        return restructure(L("Column Deletion"), .columns, insert: false, runs: [(column, 1)])
    }

    /// Moves a column of the sheet through the workbook, references following it in every
    /// sheet, as cutting and inserting a column does in Excel.
    @discardableResult
    override func moveColumn(_ column: Int, to target: Int) -> Bool {
        guard workbook != nil, !isLegacyFormat, !core.info().sheetStructureLocked else { return super.moveColumn(column, to: target) }
        return changeWorkbook(L("Column Move")) { workbook, sheets, index in
            try workbook.moveColumn(sheets: sheets, index: index, from: UInt64(column), to: UInt64(target))
        }
    }

    /// Runs of consecutive rows `(first, count)`, the last first, so deleting one leaves the
    /// others where they are.
    private static func runs(of rows: [Int]) -> [(Int, Int)] {
        var runs: [(Int, Int)] = []
        for row in Set(rows).sorted() {
            if let last = runs.last, last.0 + last.1 == row {
                runs[runs.count - 1].1 += 1
            } else {
                runs.append((row, 1))
            }
        }
        return runs.reversed()
    }

    /// Inserts or deletes rows or columns of the shown sheet through the workbook, which moves
    /// references to them in every sheet. Every sheet is read first, so no formula is missed.
    /// The sheets changed take one undo step together.
    private func restructure(_ actionName: String, _ axis: SheetAxis, insert: Bool, runs: [(Int, Int)]) -> Bool {
        guard !runs.isEmpty else { return false }
        return changeWorkbook(actionName) { workbook, sheets, index in
            var changed: [UInt64] = []
            for (at, count) in runs {
                changed += insert
                    ? try workbook.insert(sheets: sheets, index: index, axis: axis, at: UInt64(at), count: UInt64(count))
                    : try workbook.delete(sheets: sheets, index: index, axis: axis, at: UInt64(at), count: UInt64(count))
            }
            return changed
        }
    }

    /// Moves rows of the shown sheet through the workbook, references following them in
    /// every sheet, as cutting and inserting rows does in Excel.
    @discardableResult
    override func moveRows(visibleRows: [Int], beforeVisibleRow target: Int) -> Bool {
        guard rowsMoveReferences else { return super.moveRows(visibleRows: visibleRows, beforeVisibleRow: target) }
        let rows = visibleRows.compactMap { core.visibleRecord(visibleRow: UInt64($0)) }
        let to = core.insertionRow(visibleRow: UInt64(target))
        return changeWorkbook(L("Row Move")) { workbook, sheets, index in
            try workbook.moveRows(sheets: sheets, index: index, rows: rows, to: to)
        }
    }

    /// Splits a column of a sheet through the workbook: references to the columns after it
    /// move in every sheet.
    override func splitColumn(_ column: Int, separator: String, regex: Bool, done: @escaping (Int) -> Void) {
        guard workbook != nil, !isLegacyFormat else { return super.splitColumn(column, separator: separator, regex: regex, done: done) }
        var added = 0
        let changed = changeWorkbook(L("Column Split")) { workbook, sheets, index in
            let split = try workbook.splitColumn(sheets: sheets, index: index, column: UInt64(column), separator: separator, regex: regex)
            added = Int(split.count)
            return split.sheets
        }
        if changed {
            done(added)
        }
    }

    /// Joins columns of a sheet through the workbook; see `splitColumn`.
    override func mergeColumns(_ columns: [Int], separator: String) {
        guard workbook != nil, !isLegacyFormat else { return super.mergeColumns(columns, separator: separator) }
        changeWorkbook(L("Column Merge")) { workbook, sheets, index in
            try workbook.mergeColumns(sheets: sheets, index: index, columns: columns.map { UInt64($0) }, separator: separator)
        }
    }

    /// Sorts the rows of a sheet into the file as Excel sorts: formulas and formats move with
    /// their rows, references from elsewhere stay where they are.
    @discardableResult
    override func applySort() -> Bool {
        guard let workbook, rowsMoveReferences, !isLegacyFormat else { return super.applySort() }
        guard !isBusy else {
            NSSound.beep()
            return false
        }
        do {
            guard try workbook.sortRows(document: core, index: UInt64(sheetIndex)) else { return false }
        } catch {
            presentEditError(error)
            return false
        }
        registerUndo()
        undoManager?.setActionName(L("Sorting"))
        _ = tableDidChange()
        post(.structure)
        return true
    }

    /// Runs a change of the shown sheet that moves references in every sheet: each sheet is
    /// read first, so no formula is missed; the sheets changed take one undo step together.
    @discardableResult
    private func changeWorkbook(_ actionName: String, _ change: (CoreWorkbook, [SheetToSave], UInt64) throws -> [UInt64]) -> Bool {
        guard let workbook, !isLegacyFormat else { return false }
        guard !isBusy else {
            NSSound.beep()
            return false
        }
        do {
            try loadEverySheet()
        } catch {
            presentEditError(error)
            return false
        }
        let sheets = loaded.map { SheetToSave(index: UInt64($0.key), document: $0.value.document) }
        var changed: [UInt64] = []
        do {
            changed = try change(workbook, sheets, UInt64(sheetIndex))
        } catch {
            presentEditError(error)
            return false
        }
        guard !changed.isEmpty else { return false }
        // The shown sheet's step is registered first, so undoing ends on it.
        let order = [UInt64(sheetIndex)] + changed.filter { $0 != UInt64(sheetIndex) }
        for sheet in order {
            guard let table = loaded[Int(sheet)]?.document else { continue }
            for _ in changed.filter({ $0 == sheet }) {
                registerUndo(of: table)
            }
        }
        undoManager?.setActionName(actionName)
        _ = tableDidChange()
        post(.structure)
        return true
    }

    /// Reads the sheets not read yet, here and now.
    private func loadEverySheet() throws {
        guard let workbook else { return }
        for (index, sheet) in sheets.enumerated() where sheet.isTable && loaded[index] == nil {
            if let read = try workbook.readSheet(index: UInt64(index), locale: Self.displayLocale, job: SheetJob()) {
                loaded[index] = adopt(read)
            }
        }
    }

    /// A sheet just read, shown in the workbook's mode.
    private func adopt(_ sheet: LoadedSheet) -> LoadedSheet {
        sheet.document.setHasHeader(hasHeader: csvMode)
        return sheet
    }

    /// The header setting is the workbook's: CSV mode on or off for every sheet.
    override func setHasHeader(_ hasHeader: Bool) {
        csvMode = hasHeader
        for sheet in loaded.values {
            sheet.document.setHasHeader(hasHeader: hasHeader)
        }
        post(.structure)
    }

    // MARK: Formulas

    override func tableDidChange() -> Bool {
        do {
            return try computeFormulas()
        } catch {
            presentEditError(error)
            return false
        }
    }

    /// Computes what depends on changed cells, and on opening what the file asks to compute;
    /// says whether the shown sheet changed.
    @discardableResult
    private func computeFormulas() throws -> Bool {
        guard let workbook, !isLegacyFormat else { return false }
        let sheets = loaded.map { SheetToSave(index: UInt64($0.key), document: $0.value.document) }
        let report = try workbook.recalculate(sheets: sheets, now: Self.serialNow())
        circularFormulas = Int(report.circular)
        formulasNotComputed = Int(report.notComputed)
        return report.sheets.contains(UInt64(sheetIndex))
    }

    /// Now as Excel counts days: from 1900, in local time.
    static func serialNow() -> Double {
        let seconds = Date().timeIntervalSince1970 + Double(TimeZone.current.secondsFromGMT())
        return seconds / 86_400 + 25_569
    }

    /// Undo and redo show the sheet whose change they undo.
    override func show(_ table: CoreDocument) {
        guard table !== core, let index = loaded.first(where: { $0.value.document === table })?.key else { return }
        loading?.job.cancel()
        loading = nil
        show(loaded[index]!, at: index)
    }

    /// The tables before and after the shown one, for Next and Previous Sheet.
    func neighbourSheet(_ step: Int) -> Int? {
        var index = sheetIndex + step
        while sheets.indices.contains(index) {
            if sheets[index].isTable, sheets[index].visibility == .visible {
                return index
            }
            index += step
        }
        return nil
    }

    // MARK: Saving

    private static let csvTypes = [
        UTType.commaSeparatedText.identifier, UTType.tabSeparatedText.identifier, UTType.utf8TabSeparatedText.identifier,
    ]

    /// A workbook saves as itself; Export and Export for Excel write the shown sheet as CSV.
    override func writableTypes(for saveOperation: NSDocument.SaveOperationType) -> [String] {
        if saveOperation == .saveToOperation {
            return Self.csvTypes
        }
        return isLegacyFormat ? [] : [fileType].compactMap { $0 }
    }

    override func write(to url: URL, ofType typeName: String) throws {
        guard UTType(typeName)?.conforms(to: .spreadsheet) ?? false else {
            // A sheet exported as CSV.
            return try super.write(to: url, ofType: typeName)
        }
        guard let workbook, !isLegacyFormat else {
            throw CocoaError(.fileWriteNoPermission)
        }
        let sheets = loaded.map { SheetToSave(index: UInt64($0.key), document: $0.value.document) }
        let bytes: Data
        do {
            bytes = try workbook.bytes(sheets: sheets)
        } catch {
            throw Self.userFacingError(error)
        }
        // NSDocument writes into a temporary file and puts it in place of the old one.
        try bytes.write(to: url)
    }

    // Save As keeps the workbook format: no CSV options in the panel. Export still offers them.
    override func saveAs(_ sender: Any?) {
        savingAsWorkbook = true
        super.saveAs(sender)
    }

    override func saveTo(_ sender: Any?) {
        savingAsWorkbook = false
        super.saveTo(sender)
    }

    override func prepareSavePanel(_ savePanel: NSSavePanel) -> Bool {
        panelSavedWorkbook = savingAsWorkbook
        savingAsWorkbook = false
        guard panelSavedWorkbook else { return super.prepareSavePanel(savePanel) }
        savePanel.allowedContentTypes = [fileType.flatMap(UTType.init) ?? .spreadsheet]
        return true
    }

    override var fileTypeFromLastRunSavePanel: String? {
        panelSavedWorkbook ? fileType : super.fileTypeFromLastRunSavePanel
    }

    override func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(save(_:)), #selector(saveAs(_:)), #selector(duplicate(_:)):
            return !isLegacyFormat && super.validateMenuItem(menuItem)
        case #selector(reopenWithEncoding(_:)), #selector(reopenWithDelimiter(_:)):
            return false
        default:
            return super.validateMenuItem(menuItem)
        }
    }

    // MARK: Locale

    /// How this Mac shows numbers and dates, which Excel follows.
    static var displayLocale: DisplayLocale {
        let locale = Locale.current
        let formatter = DateFormatter()
        formatter.locale = locale
        let pattern = DateFormatter.dateFormat(fromTemplate: "yMd", options: 0, locale: locale) ?? "M/d/y"
        return DisplayLocale(
            decimalSeparator: locale.decimalSeparator ?? ".",
            groupSeparator: locale.groupingSeparator ?? ",",
            shortDate: excelDateCode(pattern),
            monthNames: formatter.standaloneMonthSymbols,
            monthAbbreviations: formatter.shortStandaloneMonthSymbols.map { $0.trimmingCharacters(in: CharacterSet(charactersIn: ".")) },
            dayNames: formatter.weekdaySymbols,
            dayAbbreviations: formatter.shortWeekdaySymbols,
            am: formatter.amSymbol,
            pm: formatter.pmSymbol,
            trueText: L("TRUE"),
            falseText: L("FALSE")
        )
    }

    /// An ICU date pattern (`dd.MM.y`) as an Excel format code (`dd.mm.yyyy`).
    static func excelDateCode(_ pattern: String) -> String {
        var code = ""
        var quoted = false
        var previous: Character?
        for character in pattern {
            if character == "'" {
                quoted.toggle()
                code.append("\"")
            } else if quoted {
                code.append(character)
            } else {
                switch character {
                case "y", "Y", "u":
                    if previous != "y" { code += "yyyy" }
                    previous = "y"
                    continue
                case "M", "L":
                    code.append("m")
                case "d":
                    code.append("d")
                case "G":
                    break
                default:
                    code.append(character)
                }
            }
            previous = character
        }
        return code
    }
}
