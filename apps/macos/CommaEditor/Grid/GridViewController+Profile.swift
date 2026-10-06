import AppKit

// The column profile panel: statistics of the active column, and its problem cells marked
// in the table while the panel is open.

extension GridViewController {
    var isProfileVisible: Bool { !profileContainer.isHidden }

    @objc func toggleProfilePanel(_ sender: Any?) {
        profileContainer.isHidden.toggle()
        profileWidth.constant = isProfileVisible ? 280 : 0
        if isProfileVisible {
            configureProfileActions()
            refreshProfile()
        } else {
            profiledColumn = nil
            profileModel.profile = nil
            tableView.reloadData()
        }
    }

    /// Profiles the active column off the main thread; the newest request wins.
    func refreshProfile() {
        guard isProfileVisible, columnCount > 0 else { return }
        let column = activeColumn
        profileGeneration += 1
        let generation = profileGeneration
        profileModel.columnName = columnTitle(column)
        profileModel.isLoading = true
        let core = core
        DispatchQueue.global(qos: .userInitiated).async {
            let profile = try? core.profileColumn(column: UInt64(column))
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.profileGeneration else { return }
                self.profileModel.isLoading = false
                self.profileModel.profile = profile
                let changedColumn = self.profiledColumn != column
                self.profiledColumn = column
                if changedColumn || profile != nil {
                    self.redrawProfiledColumn()
                }
            }
        }
    }

    /// Why a value stands out in the profiled column, or `nil`.
    func profileIssue(of value: String, column: Int) -> String? {
        guard isProfileVisible, column == profiledColumn, let kind = profileModel.profile?.kind else { return nil }
        if !kindFits(value: classifyValue(value: value), column: kind) {
            return L("Not %@", ColumnProfileView.kindName(kind).lowercased())
        }
        if !value.isEmpty, value.trimmingCharacters(in: .whitespacesAndNewlines) != value {
            return L("Spaces around the value")
        }
        return nil
    }

    private func redrawProfiledColumn() {
        let visible = tableView.rows(in: tableView.visibleRect)
        guard visible.length > 0 else { return }
        tableView.reloadData(
            forRowIndexes: IndexSet(integersIn: visible.location..<NSMaxRange(visible)),
            columnIndexes: IndexSet(integersIn: 0..<tableView.numberOfColumns)
        )
    }

    private func configureProfileActions() {
        profileModel.onShowIssue = { [weak self] issue in
            guard let self, let column = self.profiledColumn, let kind = self.profileModel.profile?.kind else { return }
            let after = self.activeRow.map { UInt64($0) }
            if let row = self.core.findIssue(column: UInt64(column), issue: issue, kind: kind, after: after) {
                self.activate(row: Int(row), column: column)
            }
        }
        profileModel.onTrim = { [weak self] in
            guard let self, let column = self.profiledColumn, self.dataRowCount > 0 else { return }
            let range = CellRange(top: 0, left: UInt64(column), bottom: UInt64(self.dataRowCount - 1), right: UInt64(column))
            self.document.transform(range, .trim, actionName: L("Text Change"))
        }
        profileModel.onFilterValue = { [weak self] value in
            guard let self, let column = self.profiledColumn else { return }
            self.addFilter(FilterRule(column: UInt64(column), op: value.isEmpty ? .isEmpty : .equals, value: value, caseSensitive: true))
        }
    }
}
