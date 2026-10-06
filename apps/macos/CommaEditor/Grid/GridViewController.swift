import AppKit

private extension NSUserInterfaceItemIdentifier {
    static let rowNumberColumn = NSUserInterfaceItemIdentifier("rowNumber")
    static let rowNumberCell = NSUserInterfaceItemIdentifier("rowNumberCell")
    static let dataCell = NSUserInterfaceItemIdentifier("dataCell")
}

/// The table of a document. Rows are fetched from the core in blocks as they scroll into view.
///
/// Row indices here are *data rows*: when the first row is a header, data row 0 is core row 1.
/// The active cell is the selected row (when exactly one is selected) and `activeColumn`.
final class GridViewController: NSViewController {
    private let document: CSVDocument
    private let scrollView = NSScrollView()
    private let tableView = GridTableView()
    private let statusLabel = NSTextField(labelWithString: "")
    private var info: DocumentInfo
    private var rowCache: [Int: [String]] = [:]
    private var activeColumn = 0
    private var highlightedRow: Int?
    private var changeObserver: NSObjectProtocol?

    private static let cacheBlock = 128
    private static let cacheLimit = 8_192
    private static let defaultColumnWidth: CGFloat = 140
    private static let numberFormatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter
    }()

    init(document: CSVDocument) {
        self.document = document
        self.info = document.core.info()
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        if let changeObserver {
            NotificationCenter.default.removeObserver(changeObserver)
        }
    }

    private var core: CoreDocument { document.core }
    private var headerOffset: Int { info.hasHeader ? 1 : 0 }
    private var dataRowCount: Int { max(0, Int(info.rowCount) - headerOffset) }
    private var columnCount: Int { Int(info.columnCount) }

    // MARK: View

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 1100, height: 700))

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        let separator = NSBox()
        separator.boxType = .separator

        statusLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingTail
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        for subview in [scrollView, separator, statusLabel] as [NSView] {
            subview.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(subview)
        }
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: root.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            separator.topAnchor.constraint(equalTo: scrollView.bottomAnchor),
            separator.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            separator.heightAnchor.constraint(equalToConstant: 1),
            statusLabel.topAnchor.constraint(equalTo: separator.bottomAnchor, constant: 5),
            statusLabel.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 10),
            statusLabel.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -10),
            statusLabel.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -6),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        configureTableView()
        rebuildColumns()
        updateStatus()
        changeObserver = NotificationCenter.default.addObserver(
            forName: CSVDocument.didChangeNotification,
            object: document,
            queue: nil
        ) { [weak self] notification in
            MainActor.assumeIsolated {
                self?.documentDidChange(notification)
            }
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(tableView)
        if tableView.selectedRow < 0 {
            selectDataRow(0)
        }
    }

    private func configureTableView() {
        tableView.style = .plain
        tableView.rowSizeStyle = .custom
        tableView.rowHeight = 22
        tableView.intercellSpacing = NSSize(width: 1, height: 1)
        tableView.usesAlternatingRowBackgroundColors = true
        tableView.gridStyleMask = [.solidVerticalGridLineMask, .solidHorizontalGridLineMask]
        tableView.gridColor = .separatorColor
        tableView.allowsMultipleSelection = true
        tableView.allowsColumnReordering = false
        tableView.allowsColumnResizing = true
        tableView.allowsColumnSelection = false
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.headerView = NSTableHeaderView()
        tableView.dataSource = self
        tableView.delegate = self
        tableView.keyHandler = self
        tableView.target = self
        tableView.action = #selector(tableClicked(_:))
        tableView.doubleAction = #selector(tableDoubleClicked(_:))
    }

    // MARK: Columns

    private func rebuildColumns() {
        var widths: [String: CGFloat] = [:]
        for column in tableView.tableColumns {
            widths[column.identifier.rawValue] = column.width
            tableView.removeTableColumn(column)
        }

        let numbers = NSTableColumn(identifier: .rowNumberColumn)
        numbers.title = ""
        numbers.isEditable = false
        numbers.resizingMask = []
        tableView.addTableColumn(numbers)

        for index in 0..<columnCount {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(index)"))
            column.width = widths[column.identifier.rawValue] ?? Self.defaultColumnWidth
            column.minWidth = 36
            column.resizingMask = .userResizingMask
            column.headerToolTip = Self.columnLetter(index)
            tableView.addTableColumn(column)
        }
        activeColumn = min(activeColumn, max(0, columnCount - 1))
        refreshColumnTitles()
        updateRowNumberWidth()
    }

    private func refreshColumnTitles() {
        let header = info.hasHeader ? ((try? core.row(index: 0)) ?? []) : []
        for column in tableView.tableColumns {
            guard let index = dataColumn(of: column) else { continue }
            let name = index < header.count ? Self.singleLine(header[index]) : ""
            column.title = name.isEmpty ? Self.columnLetter(index) : name
        }
        tableView.headerView?.needsDisplay = true
    }

    private func updateRowNumberWidth() {
        let digits = max(3, String(dataRowCount).count)
        tableView.tableColumn(withIdentifier: .rowNumberColumn)?.width = CGFloat(digits) * 8 + 14
    }

    private func dataColumn(of tableColumn: NSTableColumn) -> Int? {
        let raw = tableColumn.identifier.rawValue
        guard raw.hasPrefix("c") else { return nil }
        return Int(raw.dropFirst())
    }

    private func dataColumn(atTableColumn index: Int) -> Int? {
        guard index >= 0, index < tableView.tableColumns.count else { return nil }
        return dataColumn(of: tableView.tableColumns[index])
    }

    private func columnTitle(_ column: Int) -> String {
        let index = column + 1
        return index < tableView.tableColumns.count ? tableView.tableColumns[index].title : Self.columnLetter(column)
    }

    // MARK: Data

    private func values(forDataRow row: Int) -> [String] {
        let coreRow = row + headerOffset
        if let cached = rowCache[coreRow] {
            return cached
        }
        if rowCache.count > Self.cacheLimit {
            rowCache.removeAll(keepingCapacity: true)
        }
        let start = coreRow - coreRow % Self.cacheBlock
        for (offset, values) in core.rows(start: UInt64(start), count: UInt64(Self.cacheBlock)).enumerated() {
            rowCache[start + offset] = values
        }
        return rowCache[coreRow] ?? []
    }

    private func value(dataRow row: Int, column: Int) -> String {
        let values = values(forDataRow: row)
        return column < values.count ? values[column] : ""
    }

    private func documentDidChange(_ notification: Notification) {
        let previous = info
        info = core.info()
        let change = notification.userInfo?[CSVDocument.changeKey] as? CSVDocument.Change
        let sameShape = previous.rowCount == info.rowCount
            && previous.columnCount == info.columnCount
            && previous.hasHeader == info.hasHeader

        if case .cell(let row, _)? = change, sameShape {
            rowCache[row] = nil
            if info.hasHeader && row == 0 {
                refreshColumnTitles()
            } else {
                tableView.reloadData(
                    forRowIndexes: IndexSet(integer: row - headerOffset),
                    columnIndexes: IndexSet(integersIn: 0..<tableView.numberOfColumns)
                )
            }
        } else {
            rowCache.removeAll(keepingCapacity: true)
            if previous.columnCount != info.columnCount || previous.hasHeader != info.hasHeader {
                rebuildColumns()
            } else {
                refreshColumnTitles()
                updateRowNumberWidth()
            }
            highlightedRow = nil
            tableView.reloadData()
        }
        updateStatus()
    }

    // MARK: Status bar

    private func updateStatus() {
        let delimiter = switch info.delimiter {
        case "\t": L("Tab")
        case " ": L("Space")
        default: info.delimiter
        }
        let lineEnding = switch info.lineEnding {
        case .lf: "LF"
        case .crLf: "CRLF"
        case .cr: "CR"
        }
        var parts = [
            L("Rows: %@", Self.format(dataRowCount)),
            L("Columns: %@", Self.format(columnCount)),
            info.hasBom ? L("%@ with BOM", info.encoding) : info.encoding,
            L("Delimiter: %@", delimiter),
            lineEnding,
        ]
        if info.hasHeader {
            parts.append(L("Header row"))
        }
        if info.unterminatedQuote {
            parts.append(L("Unclosed quote"))
        }
        statusLabel.stringValue = parts.joined(separator: "   ·   ")
    }

    // MARK: Selection and the active cell

    private func selectDataRow(_ row: Int) {
        guard row >= 0, row < dataRowCount else { return }
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        tableView.scrollRowToVisible(row)
    }

    private var activeRow: Int? {
        tableView.selectedRowIndexes.count == 1 ? tableView.selectedRow : nil
    }

    /// Redraws the row that shows the active cell and the row that showed it before.
    private func refreshActiveCell() {
        var rows = IndexSet()
        if let highlightedRow { rows.insert(highlightedRow) }
        if let activeRow { rows.insert(activeRow) }
        highlightedRow = activeRow
        let visible = rows.filteredIndexSet { $0 < tableView.numberOfRows }
        guard !visible.isEmpty, tableView.numberOfColumns > 1 else { return }
        tableView.reloadData(forRowIndexes: visible, columnIndexes: IndexSet(integersIn: 1..<tableView.numberOfColumns))
    }

    private func moveActiveColumn(by delta: Int) {
        guard columnCount > 0 else { return }
        activeColumn = min(max(0, activeColumn + delta), columnCount - 1)
        tableView.scrollColumnToVisible(activeColumn + 1)
        refreshActiveCell()
    }

    @objc private func tableClicked(_ sender: Any?) {
        if let column = dataColumn(atTableColumn: tableView.clickedColumn) {
            activeColumn = column
        }
        refreshActiveCell()
    }

    @objc private func tableDoubleClicked(_ sender: Any?) {
        guard let column = dataColumn(atTableColumn: tableView.clickedColumn) else { return }
        activeColumn = column
        if tableView.clickedRow < 0 {
            promptToRenameColumn(column)
        } else {
            editActiveCell()
        }
    }

    // MARK: Editing cells

    /// Starts editing the active cell, optionally replacing its text with what was typed.
    /// Values with line breaks open in a sheet instead.
    private func editActiveCell(typing text: String? = nil) {
        guard let row = activeRow, columnCount > 0 else { return }
        if Self.isMultiline(value(dataRow: row, column: activeColumn)) {
            presentCellEditor(row: row, column: activeColumn)
            return
        }
        tableView.scrollColumnToVisible(activeColumn + 1)
        tableView.editColumn(activeColumn + 1, row: row, with: nil, select: true)
        if let text, let editor = view.window?.firstResponder as? NSTextView {
            editor.insertText(text, replacementRange: editor.selectedRange())
        }
    }

    private func presentCellEditor(row: Int, column: Int) {
        let editor = CellEditorViewController(
            heading: L("Row %ld, column %@", row + 1, columnTitle(column)),
            value: value(dataRow: row, column: column)
        ) { [weak self] newValue in
            guard let self else { return }
            self.document.setCell(row: row + self.headerOffset, column: column, value: newValue)
        }
        presentAsSheet(editor)
    }

    private func promptToRenameColumn(_ column: Int) {
        guard info.hasHeader, let window = view.window else { return }
        let header = (try? core.row(index: 0)) ?? []
        let field = NSTextField(string: column < header.count ? header[column] : "")
        field.frame = NSRect(x: 0, y: 0, width: 280, height: 24)

        let alert = NSAlert()
        alert.messageText = L("Rename Column")
        alert.addButton(withTitle: L("Rename"))
        alert.addButton(withTitle: L("Cancel"))
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.document.renameColumn(column, to: field.stringValue)
        }
    }

    // MARK: Menu commands

    @objc func editCell(_ sender: Any?) {
        editActiveCell()
    }

    @objc func editCellInWindow(_ sender: Any?) {
        guard let row = activeRow, columnCount > 0 else { return }
        presentCellEditor(row: row, column: activeColumn)
    }

    @objc func insertRowAbove(_ sender: Any?) {
        insertRow(below: false)
    }

    @objc func insertRowBelow(_ sender: Any?) {
        insertRow(below: true)
    }

    private func insertRow(below: Bool) {
        let selection = tableView.selectedRowIndexes
        let row: Int
        if let first = selection.first, let last = selection.last {
            row = below ? last + 1 : first
        } else {
            row = dataRowCount
        }
        if document.insertRows(at: row + headerOffset, count: 1) {
            selectDataRow(row)
        }
    }

    @objc func deleteRows(_ sender: Any?) {
        let selection = tableView.selectedRowIndexes
        guard let first = selection.first else { return }
        if document.deleteRows(selection.map { $0 + headerOffset }) {
            selectDataRow(min(first, dataRowCount - 1))
        }
    }

    @objc func delete(_ sender: Any?) {
        deleteRows(sender)
    }

    @objc func insertColumnLeft(_ sender: Any?) {
        insertColumn(at: activeColumn)
    }

    @objc func insertColumnRight(_ sender: Any?) {
        insertColumn(at: columnCount == 0 ? 0 : activeColumn + 1)
    }

    private func insertColumn(at column: Int) {
        guard document.insertColumn(at: column) else { return }
        activeColumn = column
        tableView.scrollColumnToVisible(column + 1)
        refreshActiveCell()
    }

    @objc func deleteColumn(_ sender: Any?) {
        document.deleteColumn(at: activeColumn)
    }

    @objc func renameColumn(_ sender: Any?) {
        promptToRenameColumn(activeColumn)
    }

    @objc func toggleHeaderRow(_ sender: Any?) {
        document.setHasHeader(!info.hasHeader)
    }

    /// Copies the selected rows as tab-separated text, which spreadsheets paste as cells.
    @objc func copy(_ sender: Any?) {
        let rows = tableView.selectedRowIndexes
        guard !rows.isEmpty else { return }
        let text = rows.map { row in
            let values = values(forDataRow: row)
            return (0..<columnCount)
                .map { Self.tsvField($0 < values.count ? values[$0] : "") }
                .joined(separator: "\t")
        }.joined(separator: "\n")
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    // MARK: Helpers

    static func columnLetter(_ index: Int) -> String {
        var number = index + 1
        var letters = ""
        while number > 0 {
            let remainder = (number - 1) % 26
            letters = String(Character(UnicodeScalar(UInt8(65 + remainder)))) + letters
            number = (number - 1) / 26
        }
        return letters
    }

    private static func isMultiline(_ value: String) -> Bool {
        value.unicodeScalars.contains { $0 == "\n" || $0 == "\r" }
    }

    private static func singleLine(_ value: String) -> String {
        guard isMultiline(value) else { return value }
        return value
            .replacingOccurrences(of: "\r\n", with: " ↵ ")
            .replacingOccurrences(of: "\n", with: " ↵ ")
            .replacingOccurrences(of: "\r", with: " ↵ ")
    }

    private static func tsvField(_ value: String) -> String {
        let needsQuotes = value.unicodeScalars.contains { $0 == "\t" || $0 == "\n" || $0 == "\r" || $0 == "\"" }
        return needsQuotes ? "\"" + value.replacingOccurrences(of: "\"", with: "\"\"") + "\"" : value
    }

    private static func format(_ number: Int) -> String {
        numberFormatter.string(from: NSNumber(value: number)) ?? String(number)
    }
}

// MARK: - Table data source and delegate

extension GridViewController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        dataRowCount
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let tableColumn else { return nil }
        if tableColumn.identifier == .rowNumberColumn {
            let cell = reusableCell(.rowNumberCell)
            cell.textField?.stringValue = String(row + 1)
            return cell
        }
        guard let column = dataColumn(of: tableColumn) else { return nil }
        let cell = reusableCell(.dataCell)
        let value = value(dataRow: row, column: column)
        cell.textField?.stringValue = Self.singleLine(value)
        cell.textField?.isEditable = !Self.isMultiline(value)
        let isActive = row == activeRow && column == activeColumn
        cell.layer?.borderWidth = isActive ? 2 : 0
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        refreshActiveCell()
    }

    private func reusableCell(_ identifier: NSUserInterfaceItemIdentifier) -> NSTableCellView {
        if let cell = tableView.makeView(withIdentifier: identifier, owner: self) as? NSTableCellView {
            return cell
        }
        let isData = identifier == .dataCell
        let cell = NSTableCellView()
        cell.identifier = identifier
        cell.wantsLayer = true
        // The active cell is always on the selected row, drawn in the accent color.
        cell.layer?.borderColor = NSColor.alternateSelectedControlTextColor.cgColor

        let field = NSTextField(labelWithString: "")
        field.translatesAutoresizingMaskIntoConstraints = false
        field.lineBreakMode = .byTruncatingTail
        field.cell?.usesSingleLineMode = true
        field.cell?.allowsUndo = false
        if isData {
            field.delegate = self
        } else {
            field.alignment = .right
            field.textColor = .secondaryLabelColor
            field.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        }
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }
}

// MARK: - Inline editing

extension GridViewController: NSTextFieldDelegate {
    func controlTextDidEndEditing(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        let row = tableView.row(for: field)
        guard row >= 0, let column = dataColumn(atTableColumn: tableView.column(for: field)) else { return }

        let newValue = field.stringValue
        if newValue != value(dataRow: row, column: column) {
            document.setCell(row: row + headerOffset, column: column, value: newValue)
        }

        let movement = (notification.userInfo?[NSText.movementUserInfoKey] as? Int)
            .flatMap(NSTextMovement.init(rawValue:))
        view.window?.makeFirstResponder(tableView)
        switch movement {
        case .return?:
            selectDataRow(row + 1)
        case .tab?:
            moveActiveColumn(by: 1)
        case .backtab?:
            moveActiveColumn(by: -1)
        default:
            break
        }
    }

    /// Escape restores the original value.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)),
              let field = control as? NSTextField else { return false }
        let row = tableView.row(for: field)
        if row >= 0, let column = dataColumn(atTableColumn: tableView.column(for: field)) {
            field.abortEditing()
            field.stringValue = value(dataRow: row, column: column)
        }
        view.window?.makeFirstResponder(tableView)
        return true
    }
}

// MARK: - Keyboard

extension GridViewController: GridTableViewKeyHandler {
    func gridTableView(_ tableView: GridTableView, handleKeyDown event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting([.numericPad, .function, .capsLock])
        guard modifiers.isEmpty || modifiers == .shift else { return false }

        switch event.specialKey {
        case .carriageReturn?, .enter?:
            editActiveCell()
            return true
        case .leftArrow? where modifiers.isEmpty, .backTab?:
            moveActiveColumn(by: -1)
            return true
        case .rightArrow? where modifiers.isEmpty, .tab?:
            moveActiveColumn(by: 1)
            return true
        case .delete?, .deleteForward?:
            guard let row = activeRow, columnCount > 0 else { return false }
            document.setCell(row: row + headerOffset, column: activeColumn, value: "")
            return true
        case nil:
            // Typing on a cell starts editing it with the typed text, as in a spreadsheet.
            guard let characters = event.characters, !characters.isEmpty,
                  !characters.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
                  activeRow != nil else { return false }
            editActiveCell(typing: characters)
            return true
        default:
            return false
        }
    }
}

// MARK: - Menu validation

extension GridViewController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(deleteRows(_:)), #selector(delete(_:)), #selector(copy(_:)):
            return !tableView.selectedRowIndexes.isEmpty
        case #selector(editCell(_:)), #selector(editCellInWindow(_:)):
            return activeRow != nil && columnCount > 0
        case #selector(deleteColumn(_:)):
            return columnCount > 1
        case #selector(renameColumn(_:)):
            return info.hasHeader && columnCount > 0
        case #selector(toggleHeaderRow(_:)):
            menuItem.state = info.hasHeader ? .on : .off
            return info.rowCount > 0
        default:
            return true
        }
    }
}
