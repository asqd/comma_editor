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

    /// The sheet being read, if one is.
    var loadingSheetIndex: Int? { loading?.index }

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
            let index = Int(workbook.activeSheet())
            guard let sheet = try workbook.readSheet(index: UInt64(index), locale: Self.displayLocale, job: SheetJob()) else {
                throw CocoaError(.userCancelled)
            }
            loading?.job.cancel()
            loading = nil
            self.workbook = workbook
            self.sheets = sheets
            loaded = [index: sheet]
            sheetIndex = index
            generation += 1
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
                    self.loaded[index] = sheet
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
        replaceCore(sheet.document, keepsUndo: true)
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
