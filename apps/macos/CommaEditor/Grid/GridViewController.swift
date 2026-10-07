import AppKit
import SwiftUI

private extension NSUserInterfaceItemIdentifier {
    static let rowNumberColumn = NSUserInterfaceItemIdentifier("rowNumber")
    static let rowNumberCell = NSUserInterfaceItemIdentifier("rowNumberCell")
    static let dataCell = NSUserInterfaceItemIdentifier("dataCell")
}

/// The table of a document, with the find and filter bars above it and a status bar below.
///
/// Rows here are *visible rows*: data rows as the current sort and filters show them. They are
/// fetched from the core in blocks as they scroll into view. The grid owns the selection: a
/// rectangle of cells whose cursor is the active cell; NSTableView only draws.
final class GridViewController: NSViewController {
    let document: CSVDocument
    /// The scrolling table of data columns.
    let tableView = GridTableView()
    /// Row numbers and frozen columns at the left; scrolls vertically with `tableView`.
    let frozenTable = GridTableView()
    let frozenScrollView = NSScrollView()
    lazy var frozenWidth = frozenScrollView.widthAnchor.constraint(equalToConstant: 0)
    /// Data columns kept at the left while the table scrolls sideways.
    var frozenColumnCount = 0
    /// Data columns hidden from view; the file keeps them.
    var hiddenColumns: Set<Int> = []
    /// Column widths to use at the next rebuild, by data column, after columns moved.
    var pendingColumnWidths: [Int: CGFloat]?
    /// Lines of text each row shows; more than one wraps long values.
    var linesPerRow = 1
    /// Spaces at the ends, tabs, no-break spaces and empty values are marked.
    var showsInvisibles = false
    /// The area of both tables, which hosts the row move indicator.
    let tableArea = NSView()
    let moveIndicator = NSBox()
    var scrollObservers: [NSObjectProtocol] = []
    /// The encoding and delimiter sheet already opened by itself once.
    var askedAboutDetection = false
    let findBar = FindBarView()
    let filterBar = FilterBarView()
    let findContainer: NSView
    let filterContainer: NSView
    let headerMenu = NSMenu()
    let cellMenu = NSMenu()
    private(set) var selection: GridSelection?
    private(set) var info: DocumentInfo

    let scrollView = NSScrollView()
    private let statusLabel = NSTextField(labelWithString: "")
    private var rowCache: [Int: CachedRow] = [:]
    /// The current drag started on row numbers and selects whole rows.
    var selectingRows = false
    /// The window's toolbar, for its search field and sort and filter state.
    weak var documentToolbar: DocumentToolbar?
    /// Result of the last search, shown in the status bar.
    var searchSummary: String?
    /// Counts the queries typed into find, so a late count of an older one is dropped.
    var findGeneration = 0
    /// Exports running in the background.
    var exportsRunning = 0
    /// The panel at the right of the table: the column profile, the file repair or the record card.
    let sidePanel = NSView()
    var shownSidePanel: SidePanel?
    /// 0 while the panel is hidden.
    lazy var sidePanelWidth = sidePanel.widthAnchor.constraint(equalToConstant: 0)
    let profileModel = ColumnProfileModel()
    lazy var profileHost = NSHostingView(rootView: ColumnProfileView(model: profileModel))
    /// Column whose profile the panel shows, and a counter that drops stale results.
    var profiledColumn: Int?
    var profileGeneration = 0
    let repairModel = RepairModel()
    lazy var repairHost = NSHostingView(rootView: RepairView(model: repairModel))
    let cardModel = RecordCardModel()
    lazy var cardHost = NSHostingView(rootView: RecordCardView(model: cardModel))
    /// Broken rows by file row, marked on the row numbers.
    var problemRows: [Int: ProblemKind] = [:]
    /// The pending check for broken rows, and a counter that drops stale results.
    var problemCheck: DispatchWorkItem?
    var problemGeneration = 0
    /// Opens the repair panel from the status bar; hidden while the file has no broken rows.
    let problemsButton = NSButton()
    /// Shows that a large file is being read or sorted, with a way to cancel sorting.
    let busyIndicator = NSProgressIndicator()
    let busyLabel = NSTextField(labelWithString: "")
    let busyCancel = NSButton()
    private var changeObserver: NSObjectProtocol?

    private struct CachedRow {
        let values: [String]
        /// Row of the file, the header row counted.
        let fileRow: Int
    }

    private static let cacheBlock = 128
    private static let cacheLimit = 8_192
    static let defaultColumnWidth: CGFloat = 140
    static let baseRowHeight: CGFloat = 22
    static let lineHeight: CGFloat = 16
    private static let decimalFormatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 4
        return formatter
    }()
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
    var headerOffset: Int { info.hasHeader && info.rowCount > 0 ? 1 : 0 }
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
        statusLabel.setContentHuggingPriority(.required, for: .vertical)
        let statusBar = NSView()
        configureProblemsButton()
        let busy = configureBusyViews()
        for subview in [statusLabel, busy, problemsButton] as [NSView] {
            subview.translatesAutoresizingMaskIntoConstraints = false
            statusBar.addSubview(subview)
        }
        NSLayoutConstraint.activate([
            statusLabel.topAnchor.constraint(equalTo: statusBar.topAnchor, constant: 5),
            statusLabel.bottomAnchor.constraint(equalTo: statusBar.bottomAnchor, constant: -6),
            statusLabel.leadingAnchor.constraint(equalTo: statusBar.leadingAnchor, constant: 10),
            statusLabel.trailingAnchor.constraint(lessThanOrEqualTo: busy.leadingAnchor, constant: -10),
            busy.centerYAnchor.constraint(equalTo: statusLabel.centerYAnchor),
            busy.trailingAnchor.constraint(equalTo: problemsButton.leadingAnchor, constant: -12),
            problemsButton.centerYAnchor.constraint(equalTo: statusLabel.centerYAnchor),
            problemsButton.trailingAnchor.constraint(equalTo: statusBar.trailingAnchor, constant: -10),
        ])
        let separator = NSBox()
        separator.boxType = .separator

        let sideSeparator = NSBox()
        sideSeparator.boxType = .separator
        for subview in [sideSeparator, profileHost, repairHost, cardHost] as [NSView] {
            subview.translatesAutoresizingMaskIntoConstraints = false
            sidePanel.addSubview(subview)
        }
        NSLayoutConstraint.activate([
            sideSeparator.topAnchor.constraint(equalTo: sidePanel.topAnchor),
            sideSeparator.bottomAnchor.constraint(equalTo: sidePanel.bottomAnchor),
            sideSeparator.leadingAnchor.constraint(equalTo: sidePanel.leadingAnchor),
            sideSeparator.widthAnchor.constraint(equalToConstant: 1),
        ])
        for host in [profileHost, repairHost, cardHost] as [NSView] {
            NSLayoutConstraint.activate([
                host.topAnchor.constraint(equalTo: sidePanel.topAnchor),
                host.bottomAnchor.constraint(equalTo: sidePanel.bottomAnchor),
                host.leadingAnchor.constraint(equalTo: sideSeparator.trailingAnchor),
                host.trailingAnchor.constraint(equalTo: sidePanel.trailingAnchor),
            ])
        }
        sidePanel.isHidden = true
        configureFrozenScrollView()
        let content = tableArea
        // Marks where the frozen part ends.
        let frozenEdge = NSBox()
        frozenEdge.boxType = .custom
        frozenEdge.borderWidth = 0
        frozenEdge.fillColor = .gridColor
        for subview in [frozenScrollView, frozenEdge, scrollView, sidePanel, moveIndicator] as [NSView] {
            subview.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(subview)
        }
        moveIndicator.translatesAutoresizingMaskIntoConstraints = true
        NSLayoutConstraint.activate([
            frozenScrollView.topAnchor.constraint(equalTo: content.topAnchor),
            frozenScrollView.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            frozenScrollView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            frozenWidth,
            frozenEdge.topAnchor.constraint(equalTo: content.topAnchor),
            frozenEdge.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            frozenEdge.leadingAnchor.constraint(equalTo: frozenScrollView.trailingAnchor),
            frozenEdge.widthAnchor.constraint(equalToConstant: 1),
            scrollView.topAnchor.constraint(equalTo: content.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: frozenEdge.trailingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: sidePanel.leadingAnchor),
            sidePanel.topAnchor.constraint(equalTo: content.topAnchor),
            sidePanel.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            sidePanel.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            sidePanelWidth,
        ])

        let stack = NSStackView(views: [findContainer, filterContainer, content, separator, statusBar])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.distribution = .fill
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
        // The table area takes the height left over; the window keeps its size.
        stack.setHuggingPriority(.defaultLow, for: .vertical)
        content.heightAnchor.constraint(greaterThanOrEqualToConstant: 120).isActive = true
        // The table claims all spare height, so bars and the status bar keep their natural size.
        let fillHeight = content.heightAnchor.constraint(equalTo: stack.heightAnchor)
        fillHeight.priority = NSLayoutConstraint.Priority(260)
        fillHeight.isActive = true
        findContainer.isHidden = true
        filterContainer.isHidden = true
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        for table in tables {
            configure(table)
        }
        syncVerticalScrolling()
        configureMenus()
        findBar.delegate = self
        findBar.currentSelection = { [weak self] in
            self?.selection.flatMap { $0.isSingleCell ? nil : $0.range }
        }
        filterBar.delegate = self
        rebuildColumns()
        updateStatus()
        configureRepairActions()
        configureCardActions()
        scheduleProblemCheck(after: 0)
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

    override func viewDidLayout() {
        super.viewDidLayout()
        alignFrozenBottom()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(tableView)
        if selection == nil {
            selectCell(row: 0, column: 0)
        }
        checkDetection()
        #if DEBUG
        DebugScript.run(on: self)
        #endif
    }

    private func configure(_ table: GridTableView) {
        table.style = .plain
        table.rowSizeStyle = .custom
        table.rowHeight = Self.baseRowHeight
        table.intercellSpacing = NSSize(width: 1, height: 1)
        table.usesAlternatingRowBackgroundColors = true
        table.gridStyleMask = [.solidVerticalGridLineMask, .solidHorizontalGridLineMask]
        table.gridColor = .separatorColor
        table.selectionHighlightStyle = .none
        table.allowsMultipleSelection = false
        // Dragging a header moves the column in the file.
        table.allowsColumnReordering = true
        table.allowsColumnResizing = true
        table.allowsColumnSelection = false
        table.columnAutoresizingStyle = .noColumnAutoresizing
        let header = GridHeaderView()
        header.gridDelegate = self
        table.headerView = header
        table.dataSource = self
        table.delegate = self
        table.interaction = self
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

    /// Creates the table columns: row numbers and frozen columns in the frozen table, the rest
    /// in the scrolling one. Widths are kept by data column.
    func rebuildColumns() {
        var widths: [Int: CGFloat] = [:]
        var numberWidth: CGFloat?
        for table in tables {
            for column in table.tableColumns {
                if let index = dataColumn(of: column) {
                    widths[index] = column.width
                } else {
                    numberWidth = column.width
                }
                table.removeTableColumn(column)
            }
        }
        if let pending = pendingColumnWidths {
            widths = pending
            pendingColumnWidths = nil
        }
        frozenColumnCount = min(frozenColumnCount, max(0, columnCount - 1))
        hiddenColumns = hiddenColumns.filter { $0 < columnCount }

        let numbers = NSTableColumn(identifier: .rowNumberColumn)
        numbers.title = ""
        numbers.isEditable = false
        numbers.resizingMask = []
        numbers.headerToolTip = L("Row number in the file")
        numbers.width = numberWidth ?? 40
        frozenTable.addTableColumn(numbers)

        for index in 0..<columnCount {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(index)"))
            let cell = GridHeaderCell(textCell: "")
            cell.font = column.headerCell.font
            cell.alignment = column.headerCell.alignment
            column.headerCell = cell
            column.width = widths[index] ?? Self.defaultColumnWidth
            column.minWidth = 36
            column.resizingMask = .userResizingMask
            column.headerToolTip = Self.columnLetter(index)
            column.isHidden = hiddenColumns.contains(index)
            (index < frozenColumnCount ? frozenTable : tableView).addTableColumn(column)
        }
        refreshColumnTitles()
        updateRowNumberWidth()
    }

    private func refreshColumnTitles() {
        let header = info.hasHeader ? ((try? core.row(index: 0)) ?? []) : []
        for column in allTableColumns {
            guard let index = dataColumn(of: column) else { continue }
            let name = index < header.count ? Self.singleLine(header[index]) : ""
            column.title = name.isEmpty ? Self.columnLetter(index) : name
        }
        for table in tables {
            table.headerView?.needsDisplay = true
        }
        updateHeaderIcons()
        let titles = (0..<columnCount).map(columnTitle)
        findBar.setColumns(titles)
        filterBar.setColumns(titles)
    }

    /// Sort and filter icons in the column headers.
    private func updateHeaderIcons() {
        let spec = core.viewSpec()
        let keys = spec.sort
        for column in allTableColumns {
            guard let cell = column.headerCell as? GridHeaderCell, let index = dataColumn(of: column) else { continue }
            let key = keys.first { Int($0.column) == index }
            cell.sort = key.map { $0.ascending ? .ascending : .descending } ?? .none
            cell.isFiltered = spec.filters.contains { $0.column.map { Int($0) } == index }
        }
        let sorted = keys.first.flatMap { tableColumn(forDataColumn: Int($0.column)) }
        for table in tables {
            table.headerView?.needsDisplay = true
            table.highlightedTableColumn = sorted.flatMap { table.tableColumns.contains($0) ? $0 : nil }
        }
    }

    func updateRowNumberWidth() {
        let digits = max(3, String(Int(info.rowCount)).count)
        frozenTable.tableColumn(withIdentifier: .rowNumberColumn)?.width = CGFloat(digits) * 8 + 14
        updateFrozenWidth()
    }

    func dataColumn(of tableColumn: NSTableColumn) -> Int? {
        let raw = tableColumn.identifier.rawValue
        guard raw.hasPrefix("c") else { return nil }
        return Int(raw.dropFirst())
    }

    func dataColumn(in table: NSTableView, at index: Int) -> Int? {
        guard index >= 0, index < table.tableColumns.count else { return nil }
        return dataColumn(of: table.tableColumns[index])
    }

    func columnTitle(_ column: Int) -> String {
        tableColumn(forDataColumn: column)?.title ?? Self.columnLetter(column)
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
        if case .progress? = change {
            rowsAdded(since: previous)
            return
        }
        let sameShape = previous.rowCount == info.rowCount
            && previous.visibleRowCount == info.visibleRowCount
            && previous.columnCount == info.columnCount
            && previous.hasHeader == info.hasHeader

        switch change {
        case .cell(let row, _)? where sameShape:
            rowCache[row] = nil
            if row < tableView.numberOfRows {
                reloadRows(IndexSet(integer: row))
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
            selection = selection?.clamped(rowCount: dataRowCount, columnCount: columnCount)
            reloadTables()
            syncTableSelection()
            syncFilterBar()
        }
        updateStatus()
        refreshProfile()
        refreshCard()
        scheduleProblemCheck()
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
        if !hiddenColumns.isEmpty {
            parts.append(L("Hidden columns: %@", Self.format(hiddenColumns.count)))
        }
        if !spec.sort.isEmpty {
            let keys = spec.sort.map { columnTitle(Int($0.column)) + ($0.ascending ? " ↑" : " ↓") }
            parts.append(L("Sorted by %@", keys.joined(separator: ", ")))
        }
        if let selection, !selection.isSingleCell {
            parts.insert(contentsOf: selectionSummary(selection), at: 0)
        }
        if let searchSummary, findContainer.isHidden {
            parts.insert(searchSummary, at: 0)
        }
        documentToolbar?.update(sortActive: !spec.sort.isEmpty, filterActive: !spec.filters.isEmpty)
        updateBusyState()
        statusLabel.stringValue = parts.joined(separator: "   ·   ")
        filterBar.setSummary(
            spec.filters.isEmpty ? "" : L("Showing %@ of %@", Self.format(dataRowCount), Self.format(totalDataRowCount))
        )
    }

    // MARK: Selection

    var activeRow: Int? { selection?.cursor.row }
    var activeColumn: Int { selection?.cursor.column ?? 0 }

    /// The file row of the active cell, which survives re-sorting and filtering.
    var activeFileRow: Int? {
        activeRow.flatMap { cachedRow($0)?.fileRow }
    }

    /// Changes the selection, redraws it and, unless `scroll` is false, scrolls the cursor into view.
    func select(_ newSelection: GridSelection?, scroll: Bool = true) {
        let clamped = newSelection?.clamped(rowCount: dataRowCount, columnCount: columnCount)
        if clamped != selection {
            let previousColumn = selection?.cursor.column
            selection = clamped
            syncTableSelection()
            redrawVisibleRows()
            updateStatus()
            if clamped?.cursor.column != previousColumn {
                refreshProfile()
            }
            repairCursorMoved()
            refreshCard()
        }
        if scroll, let cursor = clamped?.cursor {
            scrollToCell(row: cursor.row, column: cursor.column)
        }
    }

    func selectCell(row: Int, column: Int) {
        select(GridSelection(GridCell(row: row, column: column)))
    }

    /// Selects a file row again after the view changed, if it is still visible.
    func reselect(fileRow: Int?) {
        let row = fileRow.flatMap { core.visibleIndex(row: UInt64($0)) }.map { Int($0) } ?? 0
        selectCell(row: row, column: activeColumn)
    }

    /// Moves the active cell, scrolling it into view.
    func activate(row: Int, column: Int) {
        selectCell(row: row, column: column)
    }

    /// NSTableView's own (invisible) selection follows the cursor row: editing needs it.
    private func syncTableSelection() {
        for table in tables {
            guard let row = activeRow else {
                table.deselectAll(nil)
                continue
            }
            if table.selectedRow != row {
                table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            }
        }
    }

    func redrawVisibleRows() {
        let visible = tableView.rows(in: tableView.visibleRect)
        guard visible.length > 0 else { return }
        reloadRows(IndexSet(integersIn: visible.location..<NSMaxRange(visible)))
    }

    /// Moves the cursor by rows and columns, or to the edge of the table. With `extend` the
    /// anchor stays and the selection grows.
    private func moveCursor(rows: Int, columns: Int, extend: Bool = false, toEdge: Bool = false) {
        guard dataRowCount > 0, columnCount > 0 else { return }
        let current = selection ?? GridSelection(GridCell(row: 0, column: 0))
        var target = current.cursor
        if toEdge {
            if rows != 0 { target.row = rows < 0 ? 0 : dataRowCount - 1 }
            if columns != 0 { target.column = shownColumn(from: columns < 0 ? 0 : columnCount - 1, step: -columns.signum()) }
        } else {
            target.row += rows
            if columns != 0 {
                // Hidden columns are skipped; at the edge the cursor stays.
                let next = shownColumn(from: target.column + columns, step: columns.signum())
                target.column = (0..<columnCount).contains(next) ? next : target.column
            }
        }
        select(extend ? GridSelection(anchor: current.anchor, cursor: target) : GridSelection(target))
    }

    /// Size of the selection, and the sum and average of its numbers.
    private func selectionSummary(_ selection: GridSelection) -> [String] {
        var parts = [L("Selected: %@ × %@", Self.format(selection.rows.count), Self.format(selection.columns.count))]
        guard selection.cellCount <= 2_000_000,
              let stats = try? core.rangeStats(range: selection.range),
              stats.numbers > 0 else { return parts }
        parts.append(L("Sum: %@", Self.format(stats.sum)))
        parts.append(L("Average: %@", Self.format(stats.sum / Double(stats.numbers))))
        return parts
    }

    // MARK: Editing cells

    /// Starts editing the active cell, optionally replacing its text with what was typed.
    /// Values with line breaks open in a sheet instead.
    private func editActiveCell(typing text: String? = nil) {
        guard let row = activeRow, columnCount > 0 else { return }
        guard !document.isIndexing else {
            NSSound.beep()
            return
        }
        if Self.isMultiline(value(row: row, column: activeColumn)) {
            presentCellEditor(row: row, column: activeColumn)
            return
        }
        guard let (table, index) = location(ofDataColumn: activeColumn) else { return }
        scrollToCell(row: row, column: activeColumn)
        table.editColumn(index, row: row, with: nil, select: true)
        // The editor starts from the real value, not from the marks of invisible characters.
        if let editor = view.window?.firstResponder as? NSTextView, editor.isFieldEditor {
            editor.string = value(row: row, column: activeColumn)
            editor.selectAll(nil)
        }
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
        let row = selection.map { below ? $0.rows.upperBound + 1 : $0.rows.lowerBound } ?? dataRowCount
        let column = activeColumn
        if document.insertRows(beforeVisibleRow: row, count: 1) {
            selectCell(row: row, column: column)
        }
    }

    @objc func deleteRows(_ sender: Any?) {
        guard let selection else { return }
        let column = activeColumn
        if document.deleteRows(visibleRows: Array(selection.rows)) {
            selectCell(row: min(selection.rows.lowerBound, dataRowCount - 1), column: column)
        }
    }

    /// Edit ▸ Delete and the Delete key clear the selected cells.
    @objc func delete(_ sender: Any?) {
        guard let selection else { return }
        document.fillRange(selection.range, value: "", actionName: L("Clear"))
    }

    @objc func insertColumnLeft(_ sender: Any?) {
        insertColumn(at: activeColumn)
    }

    @objc func insertColumnRight(_ sender: Any?) {
        insertColumn(at: columnCount == 0 ? 0 : activeColumn + 1)
    }

    private func insertColumn(at column: Int) {
        let row = activeRow ?? 0
        if document.insertColumn(at: column) {
            selectCell(row: row, column: column)
        }
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

    /// Copies the selected cells as tab-separated text, which spreadsheets paste as cells.
    @objc func copy(_ sender: Any?) {
        guard let selection, let text = try? core.copyRange(range: selection.range) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc func cut(_ sender: Any?) {
        guard let selection else { return }
        copy(sender)
        document.fillRange(selection.range, value: "", actionName: L("Cutting"))
    }

    /// Pastes cells from the top-left of the selection, growing the table if needed. A single
    /// copied value fills the whole selection.
    @objc func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        paste(text: text)
    }

    func paste(text: String) {
        let grid = parsePastedText(text: text)
        guard !grid.isEmpty else { return }
        let target = selection ?? GridSelection(GridCell(row: dataRowCount, column: 0))
        if grid.count == 1, grid[0].count == 1, !target.isSingleCell {
            document.fillRange(target.range, value: grid[0][0], actionName: L("Pasting"))
            return
        }
        guard let written = document.paste(grid, atRow: target.rows.lowerBound, column: target.columns.lowerBound) else {
            return
        }
        select(GridSelection(
            anchor: GridCell(row: Int(written.bottom), column: Int(written.right)),
            cursor: GridCell(row: Int(written.top), column: Int(written.left))
        ))
    }

    /// Copies the top row of the selection down; with one row selected, copies the row above.
    @objc func fillDown(_ sender: Any?) {
        guard let selection else { return }
        var range = selection.range
        if selection.rows.count == 1 {
            guard range.top > 0 else { return }
            range.top -= 1
        }
        document.fillDown(range)
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

    static func format(_ number: Double) -> String {
        decimalFormatter.string(from: NSNumber(value: number)) ?? String(number)
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
            let cell = reusableCell(.rowNumberCell, in: tableView)
            let fileRow = cachedRow(row)?.fileRow ?? row
            let selected = selection?.rows.contains(row) ?? false
            cell.textField?.stringValue = String(fileRow - headerOffset + 1)
            cell.textField?.textColor = selected ? .controlAccentColor : .secondaryLabelColor
            // Level with the first line of wrapped values.
            cell.wrapHeight?.constant = tableView.rowHeight - 4
            cell.wrapHeight?.isActive = linesPerRow > 1
            let problem = problemRows[fileRow]
            cell.layer?.backgroundColor = problem.map { _ in NSColor.systemRed.withAlphaComponent(0.2).cgColor }
            cell.toolTip = problem.map(RepairView.kindTitle)
            return cell
        }
        guard let column = dataColumn(of: tableColumn) else { return nil }
        let cell = reusableCell(.dataCell, in: tableView)
        let values = values(forRow: row)
        let value = column < values.count ? values[column] : ""
        // The row has no such field at all: shorter than others, or a blank line.
        let isMissing = showsInvisibles && column >= values.count
        let wraps = linesPerRow > 1
        if showsInvisibles, !isMissing {
            cell.textField?.attributedStringValue = InvisibleText.render(
                value,
                wraps: wraps,
                font: cell.textField?.font ?? .systemFont(ofSize: NSFont.systemFontSize)
            )
        } else {
            cell.textField?.stringValue = wraps ? value : Self.singleLine(value)
        }
        cell.textField?.maximumNumberOfLines = linesPerRow
        cell.textField?.cell?.usesSingleLineMode = !wraps
        cell.textField?.cell?.wraps = wraps
        // A label's cell scrolls its text by default, which never wraps.
        cell.textField?.cell?.isScrollable = !wraps
        cell.textField?.lineBreakMode = wraps ? .byWordWrapping : .byTruncatingTail
        cell.textField?.cell?.truncatesLastVisibleLine = true
        // A wrapping label needs a height to lay out more than one line.
        cell.wrapHeight?.constant = tableView.rowHeight - 4
        cell.wrapHeight?.isActive = wraps
        cell.textField?.isEditable = !Self.isMultiline(value)
        let isCursor = selection?.cursor == GridCell(row: row, column: column)
        let isSelected = selection.map { !$0.isSingleCell && $0.contains(row: row, column: column) } ?? false
        cell.layer?.borderWidth = isCursor ? 2 : 0
        cell.layer?.borderColor = NSColor.controlAccentColor.cgColor
        let issue = profileIssue(of: value, column: column)
        cell.toolTip = issue ?? (isMissing ? L("This row has no such field") : nil)
        if isSelected {
            cell.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.16).cgColor
        } else if issue != nil {
            cell.layer?.backgroundColor = NSColor.systemOrange.withAlphaComponent(0.22).cgColor
        } else if isMissing {
            cell.layer?.backgroundColor = NSColor.secondaryLabelColor.withAlphaComponent(0.12).cgColor
        } else {
            cell.layer?.backgroundColor = nil
        }
        return cell
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

    private func reusableCell(_ identifier: NSUserInterfaceItemIdentifier, in tableView: NSTableView) -> GridCellView {
        if let cell = tableView.makeView(withIdentifier: identifier, owner: self) as? GridCellView {
            return cell
        }
        let isData = identifier == .dataCell
        let cell = GridCellView()
        cell.identifier = identifier
        cell.wantsLayer = true

        let field = NSTextField(labelWithString: "")
        field.translatesAutoresizingMaskIntoConstraints = false
        field.lineBreakMode = .byTruncatingTail
        field.cell?.usesSingleLineMode = true
        field.cell?.allowsUndo = false
        cell.wrapHeight = field.heightAnchor.constraint(equalToConstant: Self.baseRowHeight)
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
        guard let field = notification.object as? NSTextField, let table = table(containing: field) else { return }
        let row = table.row(for: field)
        guard row >= 0, let column = dataColumn(in: table, at: table.column(for: field)) else { return }

        let newValue = field.stringValue
        if newValue != value(row: row, column: column) {
            document.setCell(visibleRow: row, column: column, value: newValue)
        } else if showsInvisibles {
            reloadRows(IndexSet(integer: row))
        }

        let movement = (notification.userInfo?[NSText.movementUserInfoKey] as? Int)
            .flatMap(NSTextMovement.init(rawValue:))
        view.window?.makeFirstResponder(tableView)
        switch movement {
        case .return?:
            moveCursor(rows: 1, columns: 0)
        case .tab?:
            moveCursor(rows: 0, columns: 1)
        case .backtab?:
            moveCursor(rows: 0, columns: -1)
        default:
            break
        }
    }

    /// Escape restores the original value.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)),
              let field = control as? NSTextField else { return false }
        if let table = table(containing: field), table.row(for: field) >= 0,
           let column = dataColumn(in: table, at: table.column(for: field)) {
            let row = table.row(for: field)
            field.abortEditing()
            field.stringValue = Self.singleLine(value(row: row, column: column))
            if showsInvisibles {
                reloadRows(IndexSet(integer: row))
            }
        }
        view.window?.makeFirstResponder(tableView)
        return true
    }
}

// MARK: - Keyboard and mouse

extension GridViewController: GridTableViewInteraction {
    func gridTableView(_ tableView: GridTableView, handleKeyDown event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting([.numericPad, .function, .capsLock])
        guard modifiers.subtracting([.shift, .command]).isEmpty else { return false }
        let shift = modifiers.contains(.shift)
        let command = modifiers.contains(.command)

        if event.keyCode == 53 {  // Escape: collapse the selection to the active cell.
            guard let selection, !selection.isSingleCell else { return false }
            select(GridSelection(selection.cursor))
            return true
        }
        let page = max(1, tableView.rows(in: tableView.visibleRect).length - 1)
        switch event.specialKey {
        case .upArrow?:
            moveCursor(rows: -1, columns: 0, extend: shift, toEdge: command)
        case .downArrow?:
            moveCursor(rows: 1, columns: 0, extend: shift, toEdge: command)
        case .leftArrow?:
            moveCursor(rows: 0, columns: -1, extend: shift, toEdge: command)
        case .rightArrow?:
            moveCursor(rows: 0, columns: 1, extend: shift, toEdge: command)
        case .pageUp?:
            moveCursor(rows: -page, columns: 0, extend: shift)
        case .pageDown?:
            moveCursor(rows: page, columns: 0, extend: shift)
        case .home?:
            moveCursor(rows: -1, columns: 0, extend: shift, toEdge: true)
        case .end?:
            moveCursor(rows: 1, columns: 0, extend: shift, toEdge: true)
        case .carriageReturn? where modifiers.isEmpty, .enter? where modifiers.isEmpty:
            editActiveCell()
        case .tab? where !command:
            moveCursor(rows: 0, columns: 1)
        case .backTab? where !command:
            moveCursor(rows: 0, columns: -1)
        case .delete? where !command, .deleteForward? where !command:
            delete(nil)
        case nil where !command:
            // Typing on a cell starts editing it with the typed text, as in a spreadsheet.
            guard let characters = event.characters, !characters.isEmpty,
                  !characters.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
                  activeRow != nil else { return false }
            editActiveCell(typing: characters)
        default:
            return false
        }
        return true
    }

    /// Clicking a cell selects it, Shift-click extends the selection, a click on a row number
    /// selects the whole row, a double-click edits. Pressing on the number of a selected whole
    /// row starts moving the selected rows.
    func gridTableView(
        _ tableView: GridTableView,
        mouseDownAtRow row: Int,
        tableColumn: Int,
        extend: Bool,
        clickCount: Int
    ) -> GridTableView.DragKind {
        selectingRows = tableView.tableColumns[tableColumn].identifier == .rowNumberColumn
        if selectingRows {
            if !extend, clickCount == 1, canMoveRows, let selection, selection.columns.count == columnCount,
               selection.rows.contains(row) {
                return .moveRows
            }
            let anchorRow = extend ? (selection?.anchor.row ?? row) : row
            select(GridSelection(
                anchor: GridCell(row: anchorRow, column: columnCount - 1),
                cursor: GridCell(row: row, column: 0)
            ), scroll: false)
            return .select
        }
        guard let column = dataColumn(in: tableView, at: tableColumn) else { return .select }
        let cell = GridCell(row: row, column: column)
        if extend, let selection {
            select(GridSelection(anchor: selection.anchor, cursor: cell), scroll: false)
        } else {
            select(GridSelection(cell), scroll: false)
        }
        if clickCount == 2 {
            editActiveCell()
        }
        return .select
    }

    func gridTableView(_ tableView: GridTableView, mouseDraggedWith event: NSEvent) {
        guard let selection else { return }
        self.tableView.autoscroll(with: event)
        let (row, column) = cellNear(windowPoint: event.locationInWindow)
        select(
            GridSelection(anchor: selection.anchor, cursor: GridCell(row: row, column: selectingRows ? 0 : column)),
            scroll: false
        )
    }

    func gridTableView(_ tableView: GridTableView, rowMoveDraggedWith event: NSEvent) {
        self.tableView.autoscroll(with: event)
        showMoveIndicator(at: event.locationInWindow)
    }

    func gridTableView(_ tableView: GridTableView, rowMoveEndedWith event: NSEvent) {
        hideMoveIndicator()
        dropRows(at: event.locationInWindow)
    }

    func gridTableViewSelectAll(_ tableView: GridTableView) {
        guard dataRowCount > 0, columnCount > 0 else { return }
        select(GridSelection(
            anchor: GridCell(row: dataRowCount - 1, column: columnCount - 1),
            cursor: GridCell(row: 0, column: 0)
        ), scroll: false)
    }
}

/// A table cell with a text field, which gets a fixed height while text wraps.
final class GridCellView: NSTableCellView {
    var wrapHeight: NSLayoutConstraint?
}
