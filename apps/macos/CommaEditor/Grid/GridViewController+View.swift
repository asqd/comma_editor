import AppKit

// Sorting, filtering, find and replace, and the context menus.

extension GridViewController {
    // MARK: Sorting

    func setSort(_ keys: [SortKey]) {
        var spec = core.viewSpec()
        spec.sort = keys
        applyView(spec)
    }

    /// Changes the view and keeps the active row selected if it is still shown.
    @discardableResult
    func applyView(_ spec: ViewSpec) -> Bool {
        guard !document.isIndexing else {
            NSSound.beep()
            return false
        }
        let fileRow = activeFileRow
        do {
            try document.setView(spec) { [weak self] in
                self?.reselect(fileRow: fileRow)
            }
        } catch {
            _ = presentError(CSVDocument.userFacingError(error))
            return false
        }
        return true
    }

    @objc func sortAscending(_ sender: Any?) {
        setSort([SortKey(column: UInt64(activeColumn), ascending: true)])
    }

    @objc func sortDescending(_ sender: Any?) {
        setSort([SortKey(column: UInt64(activeColumn), ascending: false)])
    }

    @objc func clearSort(_ sender: Any?) {
        setSort([])
    }

    /// Makes the shown order the order of rows in the file.
    @objc func applySortToFile(_ sender: Any?) {
        let fileRow = activeFileRow
        document.applySort()
        reselect(fileRow: fileRow)
    }

    // MARK: Filtering

    /// While the filter bar is shown, its conditions filter the rows; hiding it shows them all.
    @objc func toggleFilterBar(_ sender: Any?) {
        if filterContainer.isHidden {
            filterContainer.isHidden = false
            filterBar.focusFirstRow()
        } else {
            hideFilterBar()
        }
    }

    @objc func clearFilter(_ sender: Any?) {
        hideFilterBar()
    }

    private func hideFilterBar() {
        filterContainer.isHidden = true
        filterBar.setRules([])
        var spec = core.viewSpec()
        if !spec.filters.isEmpty {
            spec.filters = []
            applyView(spec)
        }
        view.window?.makeFirstResponder(tableView)
    }

    func addFilter(_ rule: FilterRule) {
        var spec = core.viewSpec()
        spec.filters.append(rule)
        if applyView(spec) {
            filterContainer.isHidden = false
        }
    }

    /// Shows the document's filters in the bar unless the bar already says the same.
    func syncFilterBar() {
        let filters = core.viewSpec().filters
        if filterBar.rules != filters {
            filterBar.setRules(filters)
        }
        if !filters.isEmpty {
            filterContainer.isHidden = false
        }
    }

    @objc private func showOnlyValue(_ sender: NSMenuItem) {
        guard let value = sender.representedObject as? String else { return }
        addFilter(FilterRule(column: UInt64(activeColumn), op: value.isEmpty ? .isEmpty : .equals, value: value, caseSensitive: true))
    }

    @objc private func hideValue(_ sender: NSMenuItem) {
        guard let value = sender.representedObject as? String else { return }
        addFilter(FilterRule(column: UInt64(activeColumn), op: value.isEmpty ? .isNotEmpty : .notEquals, value: value, caseSensitive: true))
    }

    // MARK: Find and replace

    /// ⌘F: the toolbar's search field, or the find bar when the toolbar has none.
    @objc func showFindBar(_ sender: Any?) {
        if findContainer.isHidden, documentToolbar?.beginSearch() == true {
            return
        }
        showFindBar(replace: false)
    }

    @objc func showFindAndReplaceBar(_ sender: Any?) {
        showFindBar(replace: true)
    }

    func focusTable() {
        view.window?.makeFirstResponder(tableView)
    }

    /// Typing in the toolbar's search field searches as you type.
    func toolbarSearchChanged(_ text: String) {
        findBar.searchField.stringValue = text
        updateFindResults(jump: true)
    }

    private func showFindBar(replace: Bool) {
        if let field = documentToolbar?.searchField, !field.stringValue.isEmpty {
            findBar.searchField.stringValue = field.stringValue
        }
        // With several cells selected, find and replace work inside them.
        if let selection, !selection.isSingleCell {
            findBar.setSelectionScope(selection.range)
        }
        findContainer.isHidden = false
        updateStatus()
        findBar.isReplaceVisible = replace
        view.window?.makeFirstResponder(findBar.searchField)
        findBar.searchField.selectText(nil)
    }

    @objc func findNextMatch(_ sender: Any?) {
        findMatch(forward: true)
    }

    @objc func findPreviousMatch(_ sender: Any?) {
        findMatch(forward: false)
    }

    private var activePosition: CellPosition? {
        activeRow.map { CellPosition(row: UInt64($0), column: UInt64(activeColumn)) }
    }

    private func findMatch(forward: Bool, from position: CellPosition? = nil) {
        let query = findBar.query
        guard !query.text.isEmpty || query.wholeCell else {
            showFindBar(nil)
            return
        }
        do {
            if let match = try core.find(query: query, from: position ?? activePosition, forward: forward) {
                activate(row: Int(match.row), column: Int(match.column))
            } else {
                NSSound.beep()
            }
        } catch {
            findBar.showResult(L("Invalid regular expression"), isError: true)
        }
    }

    /// Counts the matches and moves to the first one at or after the active cell.
    private func updateFindResults(jump: Bool) {
        let query = findBar.query
        guard !query.text.isEmpty || query.wholeCell else {
            showFindResult(nil)
            return
        }
        do {
            let count = Int(try core.countMatches(query: query))
            showFindResult(count == 0 ? L("Not found") : L("Matches: %@", Self.format(count)))
            if count > 0 && jump {
                findMatch(forward: true, from: positionBeforeActiveCell)
            }
        } catch {
            showFindResult(L("Invalid regular expression"), isError: true)
        }
    }

    /// The search result goes to the find bar and, while the bar is hidden, to the status bar.
    private func showFindResult(_ text: String?, isError: Bool = false) {
        findBar.showResult(text ?? "", isError: isError)
        searchSummary = text
        updateStatus()
    }

    /// The cell just before the active one, so that a search can start at the active cell itself.
    private var positionBeforeActiveCell: CellPosition? {
        guard let row = activeRow else { return nil }
        if activeColumn > 0 {
            return CellPosition(row: UInt64(row), column: UInt64(activeColumn - 1))
        }
        if row > 0 {
            return CellPosition(row: UInt64(row - 1), column: UInt64(max(0, columnCount - 1)))
        }
        return nil
    }

    // MARK: Context menus

    func configureMenus() {
        headerMenu.delegate = self
        cellMenu.delegate = self
        for table in tables {
            table.headerView?.menu = headerMenu
            table.menu = cellMenu
        }

        let items: [(String, Selector?)] = [
            (L("Sort Ascending"), #selector(sortAscending(_:))),
            (L("Sort Descending"), #selector(sortDescending(_:))),
            (L("Clear Sort"), #selector(clearSort(_:))),
            ("", nil),
            (L("Rename Column…"), #selector(renameColumn(_:))),
            (L("Insert Column Left"), #selector(insertColumnLeft(_:))),
            (L("Insert Column Right"), #selector(insertColumnRight(_:))),
            (L("Delete Column"), #selector(deleteColumn(_:))),
            ("", nil),
            (L("Fit Column Width"), #selector(autofitColumns(_:))),
            (L("Hide Column"), #selector(hideColumns(_:))),
            (L("Show All Columns"), #selector(showAllColumns(_:))),
            (L("Freeze Columns Through This One"), #selector(freezeColumns(_:))),
            (L("Unfreeze Columns"), #selector(unfreezeColumns(_:))),
        ]
        for (title, action) in items {
            headerMenu.addItem(action == nil ? .separator() : menuItem(title, action))
        }
    }

    private func menuItem(_ title: String, _ action: Selector?, value: String? = nil) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        item.representedObject = value
        return item
    }
}

// MARK: - Menu delegate

extension GridViewController: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        if menu === headerMenu {
            guard let event = NSApp.currentEvent else { return }
            for table in tables {
                guard let headerView = table.headerView else { continue }
                let point = headerView.convert(event.locationInWindow, from: nil)
                if headerView.bounds.contains(point), let column = dataColumn(in: table, at: headerView.column(at: point)) {
                    selectCell(row: activeRow ?? 0, column: column)
                }
            }
            return
        }

        menu.removeAllItems()
        guard let table = tables.first(where: { $0.clickedRow >= 0 }),
              let column = dataColumn(in: table, at: table.clickedColumn) else { return }
        let row = table.clickedRow
        if selection?.contains(row: row, column: column) != true {
            select(GridSelection(GridCell(row: row, column: column)), scroll: false)
        }

        let value = value(row: row, column: column)
        var shown = Self.singleLine(value)
        if shown.count > 30 {
            shown = String(shown.prefix(30)) + "…"
        }
        if value.isEmpty {
            menu.addItem(menuItem(L("Show Only Empty"), #selector(showOnlyValue(_:)), value: value))
            menu.addItem(menuItem(L("Hide Empty"), #selector(hideValue(_:)), value: value))
        } else {
            menu.addItem(menuItem(L("Show Only “%@”", shown), #selector(showOnlyValue(_:)), value: value))
            menu.addItem(menuItem(L("Hide “%@”", shown), #selector(hideValue(_:)), value: value))
        }
        menu.addItem(.separator())
        menu.addItem(menuItem(L("Edit Cell"), #selector(editCell(_:))))
        menu.addItem(menuItem(L("Cut"), #selector(cut(_:))))
        menu.addItem(menuItem(L("Copy"), #selector(copy(_:))))
        menu.addItem(menuItem(L("Paste"), #selector(paste(_:))))
        menu.addItem(.separator())
        menu.addItem(menuItem(L("Insert Row Above"), #selector(insertRowAbove(_:))))
        menu.addItem(menuItem(L("Insert Row Below"), #selector(insertRowBelow(_:))))
        menu.addItem(menuItem(L("Delete Rows"), #selector(deleteRows(_:))))
    }
}

// MARK: - Bars

extension GridViewController: FindBarDelegate {
    func findBarQueryDidChange(_ bar: FindBarView) {
        documentToolbar?.searchField?.stringValue = bar.searchField.stringValue
        updateFindResults(jump: true)
    }

    func findBar(_ bar: FindBarView, findForward forward: Bool) {
        findMatch(forward: forward)
    }

    /// Replaces in the active cell if it matches, then moves to the next match.
    func findBarReplace(_ bar: FindBarView) {
        if let row = activeRow {
            document.replaceInCell(
                visibleRow: row,
                column: activeColumn,
                query: bar.query,
                replacement: bar.replaceField.stringValue
            )
        }
        updateFindResults(jump: false)
        findMatch(forward: true)
    }

    func findBarReplaceAll(_ bar: FindBarView) {
        let count = document.replaceAll(query: bar.query, replacement: bar.replaceField.stringValue)
        bar.showResult(L("Replaced: %@", Self.format(count)))
    }

    func findBarClose(_ bar: FindBarView) {
        findContainer.isHidden = true
        updateStatus()
        view.window?.makeFirstResponder(tableView)
    }
}

extension GridViewController: FilterBarDelegate {
    func filterBarDidChange(_ bar: FilterBarView) {
        var spec = core.viewSpec()
        guard spec.filters != bar.rules else { return }
        spec.filters = bar.rules
        guard !document.isIndexing else {
            bar.setSummary(L("The file is still being read"))
            return
        }
        let fileRow = activeFileRow
        do {
            try document.setView(spec) { [weak self] in
                self?.reselect(fileRow: fileRow)
            }
        } catch {
            bar.setSummary(L("Invalid regular expression"))
        }
    }

    func filterBarClose(_ bar: FilterBarView) {
        hideFilterBar()
    }
}

// MARK: - Column header icons

extension GridViewController: GridHeaderViewDelegate {
    func gridHeaderView(_ headerView: GridHeaderView, didClickFilterOf tableColumn: Int) {
        guard let table = headerView.tableView, let column = dataColumn(in: table, at: tableColumn) else { return }
        filterContainer.isHidden = false
        filterBar.beginRule(column: column)
        updateStatus()
    }
}

// MARK: - Menu and toolbar validation

extension GridViewController: NSToolbarItemValidation {
    func validateToolbarItem(_ item: NSToolbarItem) -> Bool {
        let menuItem = NSMenuItem(title: "", action: item.action, keyEquivalent: "")
        return validateMenuItem(menuItem)
    }
}

extension GridViewController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if document.isIndexing, let action = menuItem.action, Self.changingActions.contains(action) {
            return false
        }
        let spec = core.viewSpec()
        switch menuItem.action {
        case #selector(deleteRows(_:)), #selector(delete(_:)), #selector(copy(_:)), #selector(cut(_:)):
            return selection != nil
        case #selector(trimSpaces(_:)), #selector(collapseSpaces(_:)), #selector(makeUppercase(_:)),
             #selector(makeLowercase(_:)), #selector(capitalizeWords(_:)), #selector(fillEmptyFromAbove(_:)),
             #selector(fillEmptyWithValue(_:)), #selector(splitColumn(_:)), #selector(removeDuplicateRows(_:)):
            return selection != nil
        case #selector(computedColumn(_:)):
            return columnCount > 0
        case #selector(mergeColumns(_:)):
            return (selection?.columns.count ?? 0) > 1
        case #selector(toggleProfilePanel(_:)):
            menuItem.title = isProfileVisible ? L("Hide Column Profile") : L("Show Column Profile")
            return columnCount > 0
        case #selector(toggleRepairPanel(_:)):
            menuItem.title = isRepairVisible ? L("Hide File Repair") : L("Show File Repair")
            return true
        case #selector(freezeColumns(_:)):
            return columnCount > 1
        case #selector(unfreezeColumns(_:)):
            return frozenColumnCount > 0
        case #selector(hideColumns(_:)):
            return columnCount - hiddenColumns.count > (selection?.columns.count ?? 1)
        case #selector(showAllColumns(_:)):
            menuItem.title = hiddenColumns.isEmpty
                ? L("Show All Columns")
                : L("Show All Columns (%@ hidden)", Self.format(hiddenColumns.count))
            return !hiddenColumns.isEmpty
        case #selector(autofitColumns(_:)):
            return columnCount > 0
        case #selector(toggleInvisibles(_:)):
            menuItem.state = showsInvisibles ? .on : .off
            return true
        case #selector(setLinesPerRow(_:)):
            menuItem.state = menuItem.tag == linesPerRow ? .on : .off
            return true
        case #selector(moveRowsUp(_:)):
            return canMoveRows && (selection?.rows.lowerBound ?? 0) > 0
        case #selector(moveRowsDown(_:)):
            return canMoveRows && selection.map { $0.rows.upperBound < dataRowCount - 1 } ?? false
        case #selector(moveColumnLeft(_:)):
            return activeColumn > 0
        case #selector(moveColumnRight(_:)):
            return activeColumn < columnCount - 1
        case #selector(toggleRecordCard(_:)):
            menuItem.title = isCardVisible ? L("Hide Record Card") : L("Show Record Card")
            return columnCount > 0
        case #selector(previousRecord(_:)):
            return (activeRow ?? 0) > 0
        case #selector(nextRecord(_:)):
            return activeRow.map { $0 < dataRowCount - 1 } ?? false
        case #selector(copyAs(_:)):
            return selection != nil
        case #selector(exportAs(_:)):
            return columnCount > 0
        case #selector(paste(_:)):
            return NSPasteboard.general.string(forType: .string) != nil
        case #selector(fillDown(_:)):
            return selection.map { $0.rows.count > 1 || $0.rows.lowerBound > 0 } ?? false
        case #selector(insertRowAbove(_:)), #selector(insertRowBelow(_:)):
            // A new empty row would jump away or be filtered out at once.
            return spec.sort.isEmpty && spec.filters.isEmpty
        case #selector(editCell(_:)), #selector(editCellInWindow(_:)):
            return activeRow != nil && columnCount > 0
        case #selector(deleteColumn(_:)):
            return columnCount > 1
        case #selector(renameColumn(_:)):
            return info.hasHeader && columnCount > 0
        case #selector(toggleHeaderRow(_:)):
            menuItem.state = info.hasHeader ? .on : .off
            return info.rowCount > 0
        case #selector(sortAscending(_:)), #selector(sortDescending(_:)):
            return columnCount > 0
        case #selector(clearSort(_:)):
            return !spec.sort.isEmpty
        case #selector(applySortToFile(_:)):
            return !spec.sort.isEmpty && spec.filters.isEmpty
        case #selector(toggleFilterBar(_:)):
            menuItem.title = filterContainer.isHidden ? L("Show Filter") : L("Hide Filter")
            return true
        case #selector(clearFilter(_:)):
            return !spec.filters.isEmpty
        default:
            return true
        }
    }
}
