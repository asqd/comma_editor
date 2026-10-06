import AppKit

private extension NSUserInterfaceItemIdentifier {
    static let rowNumberColumn = NSUserInterfaceItemIdentifier("rowNumber")
    static let rowNumberCell = NSUserInterfaceItemIdentifier("rowNumberCell")
    static let dataCell = NSUserInterfaceItemIdentifier("dataCell")
}

/// The table of a document, with the find and filter bars above it and a status bar below.
///
/// Rows here are *visible rows*: data rows as the current sort and filters show them. They are
/// fetched from the core in blocks as they scroll into view. The active cell is the selected row
/// (when exactly one is selected) and `activeColumn`.
final class GridViewController: NSViewController {
    let document: CSVDocument
    let tableView = GridTableView()
    let findBar = FindBarView()
    let filterBar = FilterBarView()
    let findContainer: NSView
    let filterContainer: NSView
    let headerMenu = NSMenu()
    let cellMenu = NSMenu()
    var activeColumn = 0
    private(set) var info: DocumentInfo

    private let scrollView = NSScrollView()
    private let statusLabel = NSTextField(labelWithString: "")
    private var rowCache: [Int: CachedRow] = [:]
    private var highlightedRow: Int?
    private var changeObserver: NSObjectProtocol?

    private struct CachedRow {
        let values: [String]
        /// Row of the file, the header row counted.
        let fileRow: Int
    }

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
        findContainer = Self.withBottomSeparator(findBar)
        filterContainer = Self.withBottomSeparator(filterBar)
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

    var core: CoreDocument { document.core }
    var dataRowCount: Int { Int(info.visibleRowCount) }
    var columnCount: Int { Int(info.columnCount) }
    private var headerOffset: Int { info.hasHeader && info.rowCount > 0 ? 1 : 0 }
    /// Data rows in the file, shown or not.
    var totalDataRowCount: Int { Int(info.rowCount) - headerOffset }

    // MARK: View

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 1100, height: 700))

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        statusLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingTail
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let statusBar = NSView()
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusBar.addSubview(statusLabel)
        NSLayoutConstraint.activate([
            statusLabel.topAnchor.constraint(equalTo: statusBar.topAnchor, constant: 5),
            statusLabel.bottomAnchor.constraint(equalTo: statusBar.bottomAnchor, constant: -6),
            statusLabel.leadingAnchor.constraint(equalTo: statusBar.leadingAnchor, constant: 10),
            statusLabel.trailingAnchor.constraint(lessThanOrEqualTo: statusBar.trailingAnchor, constant: -10),
        ])
        let separator = NSBox()
        separator.boxType = .separator

        let stack = NSStackView(views: [findContainer, filterContainer, scrollView, separator, statusBar])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: root.topAnchor),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor),
        ])
        for view in stack.arrangedSubviews {
            view.leadingAnchor.constraint(equalTo: stack.leadingAnchor).isActive = true
            view.trailingAnchor.constraint(equalTo: stack.trailingAnchor).isActive = true
            view.setContentHuggingPriority(.required, for: .vertical)
        }
        scrollView.setContentHuggingPriority(.defaultLow, for: .vertical)
        findContainer.isHidden = true
        filterContainer.isHidden = true
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        configureTableView()
        configureMenus()
        findBar.delegate = self
        filterBar.delegate = self
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
        #if DEBUG
        DebugScript.run(on: self)
        #endif
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

    private static func withBottomSeparator(_ content: NSView) -> NSView {
        let container = NSView()
        let separator = NSBox()
        separator.boxType = .separator
        for view in [content, separator] {
            view.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(view)
        }
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: container.topAnchor),
            content.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            separator.topAnchor.constraint(equalTo: content.bottomAnchor),
            separator.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            separator.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        return container
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
        numbers.headerToolTip = L("Row number in the file")
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
        updateSortIndicators()
        let titles = (0..<columnCount).map(columnTitle)
        findBar.setColumns(titles)
        filterBar.setColumns(titles)
    }

    private func updateSortIndicators() {
        let keys = core.viewSpec().sort
        for column in tableView.tableColumns {
            tableView.setIndicatorImage(nil, in: column)
        }
        for key in keys {
            let index = Int(key.column) + 1
            guard index < tableView.tableColumns.count else { continue }
            let name = key.ascending ? "NSAscendingSortIndicator" : "NSDescendingSortIndicator"
            tableView.setIndicatorImage(NSImage(named: NSImage.Name(name)), in: tableView.tableColumns[index])
        }
        tableView.highlightedTableColumn = keys.first.flatMap { key in
            let index = Int(key.column) + 1
            return index < tableView.tableColumns.count ? tableView.tableColumns[index] : nil
        }
    }

    private func updateRowNumberWidth() {
        let digits = max(3, String(Int(info.rowCount)).count)
        tableView.tableColumn(withIdentifier: .rowNumberColumn)?.width = CGFloat(digits) * 8 + 14
    }

    func dataColumn(of tableColumn: NSTableColumn) -> Int? {
        let raw = tableColumn.identifier.rawValue
        guard raw.hasPrefix("c") else { return nil }
        return Int(raw.dropFirst())
    }

    func dataColumn(atTableColumn index: Int) -> Int? {
        guard index >= 0, index < tableView.tableColumns.count else { return nil }
        return dataColumn(of: tableView.tableColumns[index])
    }

    func columnTitle(_ column: Int) -> String {
        let index = column + 1
        return index < tableView.tableColumns.count ? tableView.tableColumns[index].title : Self.columnLetter(column)
    }

    // MARK: Data

    private func cachedRow(_ row: Int) -> CachedRow? {
        if let cached = rowCache[row] {
            return cached
        }
        guard row >= 0, row < dataRowCount else { return nil }
        if rowCache.count > Self.cacheLimit {
            rowCache.removeAll(keepingCapacity: true)
        }
        let start = row - row % Self.cacheBlock
        for (offset, values) in core.visibleRows(start: UInt64(start), count: UInt64(Self.cacheBlock)).enumerated() {
            let visibleRow = start + offset
            let fileRow = core.visibleRecord(visibleRow: UInt64(visibleRow)).map { Int($0) } ?? visibleRow
            rowCache[visibleRow] = CachedRow(values: values, fileRow: fileRow)
        }
        return rowCache[row]
    }

    func values(forRow row: Int) -> [String] {
        cachedRow(row)?.values ?? []
    }

    func value(row: Int, column: Int) -> String {
        let values = values(forRow: row)
        return column < values.count ? values[column] : ""
    }

    private func documentDidChange(_ notification: Notification) {
        let previous = info
        info = core.info()
        let change = notification.userInfo?[CSVDocument.changeKey] as? CSVDocument.Change
        let sameShape = previous.rowCount == info.rowCount
            && previous.visibleRowCount == info.visibleRowCount
            && previous.columnCount == info.columnCount
            && previous.hasHeader == info.hasHeader

        switch change {
        case .cell(let row, _)? where sameShape:
            rowCache[row] = nil
            if row < tableView.numberOfRows {
                tableView.reloadData(
                    forRowIndexes: IndexSet(integer: row),
                    columnIndexes: IndexSet(integersIn: 0..<tableView.numberOfColumns)
                )
            }
        case .header? where sameShape:
            refreshColumnTitles()
        default:
            rowCache.removeAll(keepingCapacity: true)
            if previous.columnCount != info.columnCount || previous.hasHeader != info.hasHeader {
                rebuildColumns()
            } else {
                refreshColumnTitles()
                updateRowNumberWidth()
            }
            highlightedRow = nil
            tableView.reloadData()
            syncFilterBar()
        }
        updateStatus()
    }

    // MARK: Status bar

    func updateStatus() {
        let spec = core.viewSpec()
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
        var parts: [String] = []
        if spec.filters.isEmpty {
            parts.append(L("Rows: %@", Self.format(dataRowCount)))
        } else {
            parts.append(L("Showing %@ of %@", Self.format(dataRowCount), Self.format(totalDataRowCount)))
        }
        parts += [
            L("Columns: %@", Self.format(columnCount)),
            info.hasBom ? L("%@ with BOM", info.encoding) : info.encoding,
            L("Delimiter: %@", delimiter),
            lineEnding,
        ]
        if !spec.sort.isEmpty {
            let keys = spec.sort.map { columnTitle(Int($0.column)) + ($0.ascending ? " ↑" : " ↓") }
            parts.append(L("Sorted by %@", keys.joined(separator: ", ")))
        }
        if info.unterminatedQuote {
            parts.append(L("Unclosed quote"))
        }
        statusLabel.stringValue = parts.joined(separator: "   ·   ")
        filterBar.setSummary(
            spec.filters.isEmpty ? "" : L("Showing %@ of %@", Self.format(dataRowCount), Self.format(totalDataRowCount))
        )
    }

    // MARK: Selection and the active cell

    func selectDataRow(_ row: Int) {
        guard row >= 0, row < dataRowCount else { return }
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        tableView.scrollRowToVisible(row)
    }

    var activeRow: Int? {
        tableView.selectedRowIndexes.count == 1 ? tableView.selectedRow : nil
    }

    /// The file row of the active cell, which survives re-sorting and filtering.
    var activeFileRow: Int? {
        activeRow.flatMap { cachedRow($0)?.fileRow }
    }

    /// Selects a file row again after the view changed, if it is still visible.
    func reselect(fileRow: Int?) {
        guard let fileRow, let row = core.visibleIndex(row: UInt64(fileRow)) else {
            tableView.deselectAll(nil)
            selectDataRow(0)
            return
        }
        selectDataRow(Int(row))
    }

    /// Moves the active cell, scrolling it into view.
    func activate(row: Int, column: Int) {
        activeColumn = column
        selectDataRow(row)
        tableView.scrollColumnToVisible(column + 1)
        refreshActiveCell()
    }

    /// Redraws the row that shows the active cell and the row that showed it before.
    func refreshActiveCell() {
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
        guard tableView.clickedRow >= 0,
              let column = dataColumn(atTableColumn: tableView.clickedColumn) else { return }
        activeColumn = column
        editActiveCell()
    }

    // MARK: Editing cells

    /// Starts editing the active cell, optionally replacing its text with what was typed.
    /// Values with line breaks open in a sheet instead.
    private func editActiveCell(typing text: String? = nil) {
        guard let row = activeRow, columnCount > 0 else { return }
        if Self.isMultiline(value(row: row, column: activeColumn)) {
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
            value: value(row: row, column: column)
        ) { [weak self] newValue in
            self?.document.setCell(visibleRow: row, column: column, value: newValue)
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
        if document.insertRows(beforeVisibleRow: row, count: 1) {
            selectDataRow(row)
        }
    }

    @objc func deleteRows(_ sender: Any?) {
        let selection = tableView.selectedRowIndexes
        guard let first = selection.first else { return }
        if document.deleteRows(visibleRows: Array(selection)) {
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
        let fileRow = activeFileRow
        document.setHasHeader(!info.hasHeader)
        reselect(fileRow: fileRow)
    }

    /// Copies the selected rows as tab-separated text, which spreadsheets paste as cells.
    @objc func copy(_ sender: Any?) {
        let rows = tableView.selectedRowIndexes
        guard !rows.isEmpty else { return }
        let text = rows.map { row in
            let values = values(forRow: row)
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

    static func isMultiline(_ value: String) -> Bool {
        value.unicodeScalars.contains { $0 == "\n" || $0 == "\r" }
    }

    static func singleLine(_ value: String) -> String {
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

    static func format(_ number: Int) -> String {
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
            let fileRow = cachedRow(row)?.fileRow ?? row
            cell.textField?.stringValue = String(fileRow - headerOffset + 1)
            return cell
        }
        guard let column = dataColumn(of: tableColumn) else { return nil }
        let cell = reusableCell(.dataCell)
        let value = value(row: row, column: column)
        cell.textField?.stringValue = Self.singleLine(value)
        cell.textField?.isEditable = !Self.isMultiline(value)
        let isActive = row == activeRow && column == activeColumn
        cell.layer?.borderWidth = isActive ? 2 : 0
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        refreshActiveCell()
    }

    /// Clicking a column header sorts by it: ascending, descending, then back to file order.
    /// Shift-click adds the column as a further sort key.
    func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
        guard let column = dataColumn(of: tableColumn) else { return }
        let extend = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
        var keys = core.viewSpec().sort
        if let index = keys.firstIndex(where: { $0.column == UInt64(column) }), extend || keys.count == 1 {
            if keys[index].ascending {
                keys[index].ascending = false
            } else {
                keys.remove(at: index)
            }
        } else if extend {
            keys.append(SortKey(column: UInt64(column), ascending: true))
        } else {
            keys = [SortKey(column: UInt64(column), ascending: true)]
        }
        setSort(keys)
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
        if newValue != value(row: row, column: column) {
            document.setCell(visibleRow: row, column: column, value: newValue)
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
            field.stringValue = Self.singleLine(value(row: row, column: column))
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
            document.setCell(visibleRow: row, column: activeColumn, value: "")
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
