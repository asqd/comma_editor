import AppKit

// The record card: the active row as a form at the right of the table.

extension GridViewController {
    var isCardVisible: Bool { shownSidePanel == .card }

    @objc func toggleRecordCard(_ sender: Any?) {
        showSidePanel(isCardVisible ? nil : .card)
    }

    @objc func previousRecord(_ sender: Any?) {
        moveRecord(by: -1)
    }

    @objc func nextRecord(_ sender: Any?) {
        moveRecord(by: 1)
    }

    private func moveRecord(by offset: Int) {
        guard let row = activeRow else { return }
        let target = row + offset
        guard target >= 0, target < dataRowCount else { return }
        // Text typed into the card belongs to the row it was typed on.
        cardModel.commitPending()
        selectCell(row: target, column: activeColumn)
    }

    func configureCardActions() {
        cardModel.onEdit = { [weak self] edit in
            self?.document.setCell(visibleRow: edit.row, column: edit.column, value: edit.value)
        }
        cardModel.onSelectColumn = { [weak self] row, column in
            guard let self, row < self.dataRowCount, column < self.columnCount else { return }
            if self.activeRow != row || self.activeColumn != column {
                self.selectCell(row: row, column: column)
            }
        }
        cardModel.onPrevious = { [weak self] in self?.moveRecord(by: -1) }
        cardModel.onNext = { [weak self] in self?.moveRecord(by: 1) }
    }

    /// Shows the active row in the card.
    func refreshCard() {
        guard isCardVisible else { return }
        guard let row = activeRow, row < dataRowCount, columnCount > 0 else {
            cardModel.clear()
            return
        }
        if let pending = cardModel.pending, pending.row != row {
            cardModel.commitPending()
        }
        let values = values(forRow: row)
        let fileRow = activeFileRow ?? row + headerOffset
        cardModel.show(
            row: row,
            title: L("Row %@", Self.format(fileRow - headerOffset + 1)),
            position: L("%@ of %@", Self.format(row + 1), Self.format(dataRowCount)),
            fields: (0..<columnCount).map { column in
                RecordCardModel.Field(
                    row: row,
                    column: column,
                    name: columnTitle(column),
                    value: column < values.count ? values[column] : ""
                )
            },
            activeColumn: activeColumn,
            canGoBack: row > 0,
            canGoForward: row < dataRowCount - 1
        )
    }
}
