import AppKit

/// How the table looked on a sheet of a workbook, to show it the same way on coming back.
struct SheetViewState {
    var widths: [Int: CGFloat]
    var hidden: Set<Int>
    var frozen: Int
    var selection: GridSelection?
    var scroll: NSPoint
}

// Sheets of a workbook: the tabs, the layout Excel gives each sheet, and the table's state on
// each sheet as one moves between them.

extension GridViewController {
    var workbook: WorkbookDocument? { document as? WorkbookDocument }

    /// The tabs, the mode beside them and a separator above them; hidden for a delimited file.
    func configureSheetTabs() -> NSView {
        let container = NSView()
        let separator = NSBox()
        separator.boxType = .separator
        sheetMode.controlSize = .small
        sheetMode.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        sheetMode.setToolTip(L("As Excel shows the sheet: columns lettered A, B, C"), forSegment: 0)
        sheetMode.setToolTip(L("As a CSV file: the first row names the columns"), forSegment: 1)
        sheetMode.target = self
        sheetMode.action = #selector(chooseSheetMode(_:))
        for view in [sheetTabs, sheetMode, separator] {
            view.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(view)
        }
        NSLayoutConstraint.activate([
            sheetTabs.topAnchor.constraint(equalTo: container.topAnchor),
            sheetTabs.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            sheetTabs.trailingAnchor.constraint(equalTo: sheetMode.leadingAnchor, constant: -8),
            sheetMode.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
            sheetMode.centerYAnchor.constraint(equalTo: sheetTabs.centerYAnchor),
            separator.topAnchor.constraint(equalTo: sheetTabs.bottomAnchor),
            separator.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            separator.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        container.isHidden = workbook == nil
        sheetTabs.onSelect = { [weak self] index in
            self?.workbook?.showSheet(index)
        }
        return container
    }

    func updateSheetTabs() {
        guard let workbook else { return }
        sheetTabs.update(sheets: workbook.sheets, selected: workbook.sheetIndex, loading: workbook.loadingSheetIndex)
        updateSheetMode()
    }

    func updateSheetMode() {
        guard let workbook else { return }
        sheetMode.selectedSegment = workbook.csvMode ? 1 : 0
        sheetMode.isEnabled = !document.isBusy && info.rowCount > 0
    }

    /// Selections kept for the sheets not shown, moved by `rows` as their first row stops or
    /// starts being a row of the table.
    func shiftSheetSelections(by rows: Int) {
        sheetStates = sheetStates.mapValues { state in
            var state = state
            if var selection = state.selection {
                selection.anchor.row = max(0, selection.anchor.row + rows)
                selection.cursor.row = max(0, selection.cursor.row + rows)
                state.selection = selection
            }
            return state
        }
    }

    /// Before the table is first built: the layout of the sheet the workbook opens at.
    func applyFirstSheetLayout() {
        guard let workbook else { return }
        apply(initialState(for: workbook.layout))
        shownSheet = (workbook.generation, workbook.sheetIndex)
        updateSheetTabs()
    }

    /// Another sheet is shown, or the workbook was read again: the table is built anew, as it
    /// was left on that sheet or as the file lays it out.
    func sheetDidChange() {
        guard let workbook else { return }
        if let shown = shownSheet, shown.generation == workbook.generation {
            sheetStates[shown.index] = currentSheetState()
        } else {
            sheetStates = [:]
        }
        let state = sheetStates[workbook.sheetIndex] ?? initialState(for: workbook.layout)
        shownSheet = (workbook.generation, workbook.sheetIndex)
        clearRowCache()
        problemRows = [:]
        searchSummary = nil
        apply(state)
        rebuildColumns()
        reloadTables()
        select(state.selection ?? GridSelection(GridCell(row: 0, column: firstShownColumn)), scroll: false)
        scrollView.contentView.scroll(to: state.scroll)
        scrollView.reflectScrolledClipView(scrollView.contentView)
        syncFilterBar()
        updateStatus()
        refreshProfile()
        refreshCard()
        scheduleProblemCheck()
        updateSheetTabs()
    }

    private var firstShownColumn: Int {
        (0..<columnCount).first { !hiddenColumns.contains($0) } ?? 0
    }

    private func apply(_ state: SheetViewState) {
        pendingColumnWidths = state.widths
        hiddenColumns = state.hidden
        frozenColumnCount = state.frozen
    }

    private func currentSheetState() -> SheetViewState {
        var widths: [Int: CGFloat] = [:]
        for column in allTableColumns {
            if let index = dataColumn(of: column) {
                widths[index] = column.width
            }
        }
        return SheetViewState(
            widths: widths,
            hidden: hiddenColumns,
            frozen: frozenColumnCount,
            selection: selection,
            scroll: scrollView.contentView.bounds.origin
        )
    }

    /// Widths, hidden columns and frozen columns as Excel shows the sheet.
    private func initialState(for layout: SheetLayout?) -> SheetViewState {
        let columns = Int(info.columnCount)
        var state = SheetViewState(widths: [:], hidden: [], frozen: 0, selection: nil, scroll: topLeft)
        guard let layout else { return state }
        // Widths only when the sheet sets some; otherwise the table's own default.
        if layout.columns.contains(where: { $0.width != nil }) || layout.defaultColumnWidth != nil {
            let standard = Self.points(forExcelWidth: layout.defaultColumnWidth ?? 8.43)
            for index in 0..<columns {
                state.widths[index] = standard
            }
        }
        for column in layout.columns {
            let first = Int(column.first)
            let last = min(Int(column.last), columns - 1)
            guard first <= last else { continue }
            for index in first...last {
                if let width = column.width {
                    state.widths[index] = Self.points(forExcelWidth: width)
                }
                if column.hidden {
                    state.hidden.insert(index)
                }
            }
        }
        // At least one column stays in view.
        if state.hidden.count >= columns {
            state.hidden = []
        }
        state.frozen = min(Int(layout.frozenColumns), max(0, columns - 1))
        return state
    }

    /// The scroll position that shows the first row and column: the column headers lie over
    /// the top of the content.
    private var topLeft: NSPoint {
        let insets = scrollView.contentView.contentInsets
        return NSPoint(x: -insets.left, y: -insets.top)
    }

    /// Excel measures widths in digits of its default font, Calibri 11; the table's font is
    /// larger, and its cells have more padding.
    static func points(forExcelWidth width: Double) -> CGFloat {
        let pixels = width < 1 ? width * 12 : width * 7 + 5
        return min(800, max(36, CGFloat(pixels * 1.35).rounded()))
    }

    // MARK: Commands

    @objc func chooseSheetMode(_ sender: NSSegmentedControl) {
        guard (sender.selectedSegment == 1) != info.hasHeader else { return }
        guard !document.isBusy else {
            NSSound.beep()
            updateSheetMode()
            return
        }
        toggleHeaderRow(sender)
    }

    @objc func nextSheet(_ sender: Any?) {
        workbook?.neighbourSheet(1).map { workbook?.showSheet($0) }
    }

    @objc func previousSheet(_ sender: Any?) {
        workbook?.neighbourSheet(-1).map { workbook?.showSheet($0) }
    }
}
