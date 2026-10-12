import AppKit

// Frozen rows: the first rows stay in view while the table scrolls up and down. Two more tables
// lie over the top of the frozen and the scrolling table, under the column headers, and show
// the first visible rows. Rows scroll on underneath; the row under the last frozen one is the
// next row in view, as in Excel. The tables over the top use the same rows as the tables
// under them, so a row is the same visible row in all four.

/// A scroll view that does not scroll itself: wheel and trackpad go to the table under it.
/// VoiceOver reads the rows in the table under it, so it is left out for VoiceOver.
final class ForwardingScrollView: NSScrollView {
    weak var target: NSScrollView?

    override func isAccessibilityElement() -> Bool { false }

    override func accessibilityChildren() -> [Any]? { [] }

    override func scrollWheel(with event: NSEvent) {
        if let target {
            target.scrollWheel(with: event)
        } else {
            super.scrollWheel(with: event)
        }
    }
}

extension GridViewController {
    /// The tables that show the frozen rows: row numbers and frozen columns, the other columns.
    var frozenRowTables: [GridTableView] { [topFrozenTable, topTable] }

    /// Every table that shows rows.
    var rowTables: [GridTableView] { tables + frozenRowTables }

    /// Visible rows frozen at the top. A sheet freezes its own first rows, so in CSV mode,
    /// where the first row names the columns, one fewer; one row always scrolls.
    var frozenRowCount: Int {
        min(max(0, frozenTopRows - headerOffset), max(0, dataRowCount - 1))
    }

    func isFrozenRowTable(_ table: NSTableView) -> Bool {
        table === topTable || table === topFrozenTable
    }

    /// The table over the top that shows the rows of `table`.
    func frozenRowTable(over table: GridTableView) -> GridTableView {
        table === frozenTable ? topFrozenTable : topTable
    }

    // MARK: Setting up

    func configureFrozenRows(in content: NSView) {
        for (scroll, table) in [(topFrozenScrollView, topFrozenTable), (topScrollView, topTable)] {
            scroll.documentView = table
            scroll.target = scrollView
            scroll.hasVerticalScroller = false
            scroll.hasHorizontalScroller = false
            scroll.borderType = .noBorder
            scroll.drawsBackground = false
            scroll.automaticallyAdjustsContentInsets = false
            scroll.isHidden = true
        }
        frozenRowsEdge.boxType = .custom
        frozenRowsEdge.borderWidth = 0
        frozenRowsEdge.fillColor = .gridColor
        frozenRowsEdge.isHidden = true
        for view in [topFrozenScrollView, topScrollView, frozenRowsEdge] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(view, positioned: .below, relativeTo: moveIndicator)
        }
        NSLayoutConstraint.activate([
            frozenRowsTop,
            frozenRowsHeight,
            frozenRowsTrailing,
            topScrollView.leadingAnchor.constraint(equalTo: scrollView.leadingAnchor),
            topFrozenScrollView.topAnchor.constraint(equalTo: topScrollView.topAnchor),
            topFrozenScrollView.heightAnchor.constraint(equalTo: topScrollView.heightAnchor),
            topFrozenScrollView.leadingAnchor.constraint(equalTo: frozenScrollView.leadingAnchor),
            topFrozenScrollView.widthAnchor.constraint(equalTo: frozenScrollView.widthAnchor),
            frozenRowsEdge.topAnchor.constraint(equalTo: topScrollView.bottomAnchor),
            frozenRowsEdge.heightAnchor.constraint(equalToConstant: 1),
            frozenRowsEdge.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            frozenRowsEdge.trailingAnchor.constraint(equalTo: topScrollView.trailingAnchor),
        ])
    }

    /// The tables over the top follow the scrolling table sideways.
    func syncFrozenRowsScrolling() {
        let source = scrollView.contentView
        let observer = NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification,
            object: source,
            queue: nil
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let clip = self.topScrollView.contentView
                guard clip.bounds.origin.x != source.bounds.origin.x else { return }
                clip.scroll(to: NSPoint(x: source.bounds.origin.x, y: 0))
                self.topScrollView.reflectScrolledClipView(clip)
            }
        }
        scrollObservers.append(observer)
    }

    // MARK: Updating

    /// After the rows, the columns or the setting changed: how many rows, how high, which
    /// columns, as the tables under them have.
    func updateFrozenRows() {
        let count = frozenRowCount
        let hidden = count == 0
        topScrollView.isHidden = hidden
        topFrozenScrollView.isHidden = hidden
        frozenRowsEdge.isHidden = hidden
        frozenRowsHeight.constant = CGFloat(count) * (tableView.rowHeight + tableView.intercellSpacing.height)
        syncFrozenRowColumns()
        for table in frozenRowTables {
            table.rowHeight = tableView.rowHeight
            table.reloadData()
        }
        layoutFrozenRows()
    }

    /// Right under the column headers, and short of a vertical scroll bar that takes space.
    func layoutFrozenRows() {
        frozenRowsTop.constant = tableView.headerView?.frame.height ?? 0
        var scroller: CGFloat = 0
        if scrollView.scrollerStyle == .legacy, let bar = scrollView.verticalScroller, !bar.isHidden {
            scroller = bar.frame.width
        }
        frozenRowsTrailing.constant = -scroller
    }

    /// The columns of the tables over the top: those of the tables under them, as wide and
    /// as hidden.
    func syncFrozenRowColumns() {
        for (source, mirror) in [(frozenTable, topFrozenTable), (tableView, topTable)] {
            if mirror.tableColumns.map(\.identifier) != source.tableColumns.map(\.identifier) {
                for column in mirror.tableColumns {
                    mirror.removeTableColumn(column)
                }
                for column in source.tableColumns {
                    let copy = NSTableColumn(identifier: column.identifier)
                    copy.minWidth = column.minWidth
                    copy.isEditable = column.isEditable
                    copy.resizingMask = []
                    mirror.addTableColumn(copy)
                }
            }
            for (column, copy) in zip(source.tableColumns, mirror.tableColumns) {
                copy.width = column.width
                copy.isHidden = column.isHidden
            }
        }
    }

    /// Scrolls a row that is not frozen into view below the frozen ones.
    func scrollRowBelowFrozen(_ row: Int) {
        tableView.scrollRowToVisible(row)
        guard frozenRowCount > 0, row >= frozenRowCount else { return }
        let rowTop = tableView.rect(ofRow: row).minY
        let frozenBottom = tableView.convert(NSPoint(x: 0, y: topScrollView.frame.minY), from: tableArea).y
        guard rowTop < frozenBottom else { return }
        let clip = scrollView.contentView
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: clip.bounds.origin.y - (frozenBottom - rowTop)))
        scrollView.reflectScrolledClipView(clip)
    }

    /// The table over the top under a point of the window, if the point is over one.
    func frozenRowTable(at windowPoint: NSPoint) -> GridTableView? {
        guard frozenRowCount > 0 else { return nil }
        for (scroll, table) in [(topFrozenScrollView, topFrozenTable), (topScrollView, topTable)] {
            if scroll.bounds.contains(scroll.convert(windowPoint, from: nil)) {
                return table
            }
        }
        return nil
    }

    // MARK: Commands

    @objc func freezeRows(_ sender: Any?) {
        guard let row = activeRow else { return }
        frozenTopRows = min(row + 1, dataRowCount - 1) + headerOffset
        updateFrozenRows()
        scrollToCell(row: row, column: activeColumn)
    }

    @objc func unfreezeRows(_ sender: Any?) {
        frozenTopRows = 0
        updateFrozenRows()
    }
}
