import AppKit

// The grid draws with two tables: the frozen table at the left holds the row numbers and the
// frozen columns, the scrolling table the other columns. They scroll vertically together.
// Here too: hiding and fitting columns, wrapping text, and moving rows and columns.

extension GridViewController {
    var tables: [GridTableView] { [frozenTable, tableView] }

    /// Table columns of both tables, row numbers first, then data columns in order.
    var allTableColumns: [NSTableColumn] { frozenTable.tableColumns + tableView.tableColumns }

    func tableColumn(forDataColumn column: Int) -> NSTableColumn? {
        location(ofDataColumn: column).map { $0.table.tableColumns[$0.index] }
    }

    /// The table that shows a data column, and the column's index in it.
    func location(ofDataColumn column: Int) -> (table: GridTableView, index: Int)? {
        let identifier = NSUserInterfaceItemIdentifier("c\(column)")
        for table in tables {
            let index = table.column(withIdentifier: identifier)
            if index >= 0 {
                return (table, index)
            }
        }
        return nil
    }

    func table(containing view: NSView) -> GridTableView? {
        tables.first { view.isDescendant(of: $0) }
    }

    // MARK: Layout and scrolling

    func configureFrozenScrollView() {
        frozenScrollView.documentView = frozenTable
        frozenScrollView.hasVerticalScroller = false
        frozenScrollView.hasHorizontalScroller = false
        frozenScrollView.horizontalScrollElasticity = .none
        frozenScrollView.borderType = .noBorder
        frozenScrollView.automaticallyAdjustsContentInsets = false
        moveIndicator.boxType = .custom
        moveIndicator.borderWidth = 0
        moveIndicator.fillColor = .controlAccentColor
        moveIndicator.isHidden = true
    }

    /// Scrolling either table up or down scrolls the other one the same.
    func syncVerticalScrolling() {
        let pairs = [(scrollView.contentView, frozenScrollView.contentView), (frozenScrollView.contentView, scrollView.contentView)]
        for (source, target) in pairs {
            source.postsBoundsChangedNotifications = true
            let observer = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification,
                object: source,
                queue: nil
            ) { [weak target] _ in
                MainActor.assumeIsolated {
                    guard let target, target.bounds.origin.y != source.bounds.origin.y else { return }
                    target.scroll(to: NSPoint(x: target.bounds.origin.x, y: source.bounds.origin.y))
                    (target.superview as? NSScrollView)?.reflectScrolledClipView(target)
                }
            }
            scrollObservers.append(observer)
        }
    }

    /// With scroll bars that take space, the frozen table stops where the scrolling table's
    /// horizontal scroll bar starts, so their rows stay level at the bottom.
    func alignFrozenBottom() {
        var inset: CGFloat = 0
        if scrollView.scrollerStyle == .legacy, let scroller = scrollView.horizontalScroller, !scroller.isHidden {
            inset = scroller.frame.height
        }
        if frozenScrollView.contentInsets.bottom != inset {
            frozenScrollView.contentInsets = NSEdgeInsets(top: 0, left: 0, bottom: inset, right: 0)
        }
    }

    func updateFrozenWidth() {
        let spacing = frozenTable.intercellSpacing.width
        frozenWidth.constant = frozenTable.tableColumns
            .filter { !$0.isHidden }
            .reduce(0) { $0 + $1.width + spacing }
    }

    func reloadTables() {
        for table in tables {
            table.reloadData()
        }
    }

    func reloadRows(_ rows: IndexSet) {
        for table in tables where table.numberOfColumns > 0 {
            table.reloadData(forRowIndexes: rows, columnIndexes: IndexSet(integersIn: 0..<table.numberOfColumns))
        }
    }

    func scrollToCell(row: Int, column: Int) {
        if row >= 0, row < dataRowCount {
            tableView.scrollRowToVisible(row)
        }
        if let (table, index) = location(ofDataColumn: column), table === tableView {
            tableView.scrollColumnToVisible(index)
        }
    }

    /// The cell nearest to a point in the window, over either table.
    func cellNear(windowPoint: NSPoint) -> (row: Int, column: Int) {
        let inFrozen = frozenScrollView.convert(windowPoint, from: nil).x < frozenScrollView.bounds.width
        let table = inFrozen ? frozenTable : tableView
        let point = table.convert(windowPoint, from: nil)
        let row = table.nearestRow(to: point)
        let column = table.numberOfColumns > 0 ? dataColumn(in: table, at: table.nearestColumn(to: point)) : nil
        // Over the row numbers: the first column.
        return (row, column ?? 0)
    }

    /// The first shown column from `column` on in the direction of `step`; may run past the edge.
    func shownColumn(from column: Int, step: Int) -> Int {
        var column = column
        while (0..<columnCount).contains(column), hiddenColumns.contains(column) {
            column += step == 0 ? 1 : step
        }
        return column
    }

    // MARK: Freezing and hiding columns

    @objc func freezeColumns(_ sender: Any?) {
        frozenColumnCount = min(activeColumn + 1, columnCount - 1)
        rebuildColumns()
        reloadTables()
        scrollToCell(row: activeRow ?? 0, column: activeColumn)
    }

    @objc func unfreezeColumns(_ sender: Any?) {
        frozenColumnCount = 0
        rebuildColumns()
        reloadTables()
    }

    /// Hides the selected columns, or the active one. One column always stays shown.
    @objc func hideColumns(_ sender: Any?) {
        let columns = Set(selection?.columns ?? activeColumn...activeColumn)
        guard hiddenColumns.union(columns).count < columnCount else {
            NSSound.beep()
            return
        }
        hiddenColumns.formUnion(columns)
        applyHiddenColumns()
        let next = shownColumn(from: activeColumn, step: 1)
        selectCell(row: activeRow ?? 0, column: next < columnCount ? next : shownColumn(from: activeColumn, step: -1))
    }

    @objc func showAllColumns(_ sender: Any?) {
        hiddenColumns.removeAll()
        applyHiddenColumns()
    }

    private func applyHiddenColumns() {
        for column in allTableColumns {
            guard let index = dataColumn(of: column) else { continue }
            column.isHidden = hiddenColumns.contains(index)
        }
        updateFrozenWidth()
        updateStatus()
    }

    // MARK: Column widths

    /// Fits the selected columns, or all columns when one cell is selected, to their content.
    @objc func autofitColumns(_ sender: Any?) {
        let columns = selection.map { $0.isSingleCell ? 0..<columnCount : $0.columns.lowerBound..<$0.columns.upperBound + 1 }
            ?? 0..<columnCount
        for column in columns {
            tableColumn(forDataColumn: column)?.width = fittingWidth(of: column)
        }
        updateFrozenWidth()
    }

    /// Width that shows the header and the values of the rows in view and of the first rows.
    func fittingWidth(of column: Int) -> CGFloat {
        guard let tableColumn = tableColumn(forDataColumn: column) else { return Self.defaultColumnWidth }
        let headerFont = tableColumn.headerCell.font ?? .systemFont(ofSize: NSFont.smallSystemFontSize)
        // Room for the sort and filter icons.
        var width = (tableColumn.title as NSString).size(withAttributes: [.font: headerFont]).width + 44
        let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        let visible = tableView.rows(in: tableView.visibleRect)
        var rows = IndexSet(integersIn: 0..<min(dataRowCount, 300))
        rows.insert(integersIn: visible.location..<NSMaxRange(visible))
        for row in rows where row < dataRowCount {
            let text = Self.singleLine(value(row: row, column: column))
            guard !text.isEmpty else { continue }
            width = max(width, (text as NSString).size(withAttributes: [.font: font]).width + 14)
        }
        return min(max(width.rounded(.up), tableColumn.minWidth), 480)
    }

    // MARK: Invisible characters

    @objc func toggleInvisibles(_ sender: Any?) {
        showsInvisibles.toggle()
        reloadTables()
    }

    // MARK: Text wrapping

    /// View ▸ Text Wrapping: the menu item's tag is the number of lines per row.
    @objc func setLinesPerRow(_ sender: Any?) {
        guard let item = sender as? NSMenuItem, item.tag > 0, item.tag != linesPerRow else { return }
        linesPerRow = item.tag
        for table in tables {
            table.rowHeight = Self.baseRowHeight + CGFloat(linesPerRow - 1) * Self.lineHeight
        }
        reloadTables()
        if let row = activeRow {
            tableView.scrollRowToVisible(row)
        }
    }

    // MARK: Moving columns

    @objc func moveColumnLeft(_ sender: Any?) {
        moveActiveColumn(to: activeColumn - 1)
    }

    @objc func moveColumnRight(_ sender: Any?) {
        moveActiveColumn(to: activeColumn + 1)
    }

    private func moveActiveColumn(to target: Int) {
        guard (0..<columnCount).contains(target) else { return }
        moveColumn(activeColumn, to: target)
    }

    /// Moves a column in the file; widths and hidden columns move along.
    func moveColumn(_ from: Int, to target: Int) {
        var widths = (0..<columnCount).map { tableColumn(forDataColumn: $0)?.width ?? Self.defaultColumnWidth }
        var hidden = (0..<columnCount).map { hiddenColumns.contains($0) }
        widths.insert(widths.remove(at: from), at: target)
        hidden.insert(hidden.remove(at: from), at: target)
        let row = activeRow ?? 0
        pendingColumnWidths = Dictionary(uniqueKeysWithValues: widths.enumerated().map { ($0.offset, $0.element) })
        let previousHidden = hiddenColumns
        hiddenColumns = Set(hidden.indices.filter { hidden[$0] })
        let moved = document.moveColumn(from, to: target)
        if !moved {
            pendingColumnWidths = nil
            hiddenColumns = previousHidden
        }
        // The column count stays the same, so the change alone does not rebuild the columns,
        // and a dragged header already sits in its new place.
        rebuildColumns()
        reloadTables()
        selectCell(row: row, column: moved ? target : from)
    }

    // MARK: Moving rows

    /// Rows move in the order of the file, so not while the rows are sorted.
    var canMoveRows: Bool { core.viewSpec().sort.isEmpty && !document.isReadOnly && !info.sheetRowsFixed }

    @objc func moveRowsUp(_ sender: Any?) {
        guard let selection, selection.rows.lowerBound > 0 else { return }
        moveSelectedRows(before: selection.rows.lowerBound - 1)
    }

    @objc func moveRowsDown(_ sender: Any?) {
        guard let selection, selection.rows.upperBound < dataRowCount - 1 else { return }
        moveSelectedRows(before: selection.rows.upperBound + 2)
    }

    /// Moves the selected rows to stand before visible row `target`, and selects them there.
    private func moveSelectedRows(before target: Int) {
        guard let selection, canMoveRows else { return }
        let rows = selection.rows
        guard target < rows.lowerBound || target > rows.upperBound + 1 else { return }
        guard document.moveRows(visibleRows: Array(rows), beforeVisibleRow: target) else { return }
        let top = target > rows.upperBound ? target - rows.count : target
        let shift = top - rows.lowerBound
        select(GridSelection(
            anchor: GridCell(row: selection.anchor.row + shift, column: selection.anchor.column),
            cursor: GridCell(row: selection.cursor.row + shift, column: selection.cursor.column)
        ))
    }

    /// A point of the window as a point of the scrolling table at the same height. The row
    /// numbers being dragged lie left of that table, where it finds no row at all.
    private func rowPoint(_ windowPoint: NSPoint) -> NSPoint {
        NSPoint(x: tableView.visibleRect.midX, y: tableView.convert(windowPoint, from: nil).y)
    }

    /// Visible row before which rows dropped at a point go.
    private func dropTarget(at windowPoint: NSPoint) -> Int {
        let point = rowPoint(windowPoint)
        let row = tableView.row(at: point)
        guard row >= 0 else {
            return point.y < tableView.visibleRect.minY ? 0 : dataRowCount
        }
        return point.y > tableView.rect(ofRow: row).midY ? row + 1 : row
    }

    func showMoveIndicator(at windowPoint: NSPoint) {
        guard dataRowCount > 0 else { return }
        let target = dropTarget(at: windowPoint)
        let y = target < dataRowCount ? tableView.rect(ofRow: target).minY : tableView.rect(ofRow: dataRowCount - 1).maxY
        let point = tableArea.convert(NSPoint(x: 0, y: y), from: tableView)
        let visible = tableArea.convert(scrollView.contentView.frame, from: scrollView)
        guard point.y >= visible.minY - 1, point.y <= visible.maxY + 1 else {
            moveIndicator.isHidden = true
            return
        }
        moveIndicator.frame = NSRect(x: 0, y: point.y - 1, width: scrollView.frame.maxX, height: 2)
        moveIndicator.isHidden = false
    }

    func hideMoveIndicator() {
        moveIndicator.isHidden = true
    }

    /// Drops the selected rows; a click without a drag selects just the clicked row.
    func dropRows(at windowPoint: NSPoint) {
        guard let selection else { return }
        let target = dropTarget(at: windowPoint)
        if target >= selection.rows.lowerBound, target <= selection.rows.upperBound + 1 {
            let row = tableView.nearestRow(to: rowPoint(windowPoint))
            select(GridSelection(anchor: GridCell(row: row, column: columnCount - 1), cursor: GridCell(row: row, column: 0)), scroll: false)
            return
        }
        moveSelectedRows(before: target)
    }
}

// MARK: - Column resizing, fitting and dragging

extension GridViewController {
    @objc func tableViewColumnDidResize(_ notification: Notification) {
        if notification.object as? NSTableView === frozenTable {
            updateFrozenWidth()
        }
    }

    /// Double-clicking a header divider fits the column to its content.
    @objc func tableView(_ tableView: NSTableView, sizeToFitWidthOfColumn column: Int) -> CGFloat {
        let tableColumn = tableView.tableColumns[column]
        guard let index = dataColumn(of: tableColumn) else { return tableColumn.width }
        DispatchQueue.main.async { [weak self] in self?.updateFrozenWidth() }
        return fittingWidth(of: index)
    }

    /// The row number column stays first.
    @objc func tableView(_ tableView: NSTableView, shouldReorderColumn columnIndex: Int, toColumn newColumnIndex: Int) -> Bool {
        guard tableView === frozenTable else { return true }
        return columnIndex > 0 && newColumnIndex > 0
    }

    /// A header dragged to another place moves the column in the file.
    @objc func tableView(_ tableView: NSTableView, didDrag tableColumn: NSTableColumn) {
        guard let from = dataColumn(of: tableColumn) else { return }
        let index = tableView.column(withIdentifier: tableColumn.identifier)
        let target = tableView === frozenTable ? index - 1 : frozenColumnCount + index
        guard target != from, (0..<columnCount).contains(target) else { return }
        moveColumn(from, to: target)
    }
}
