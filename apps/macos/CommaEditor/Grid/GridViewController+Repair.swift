import AppKit

// File repair: broken rows are found in the background after every change, marked on the row
// numbers and listed in the repair panel, where each one can be fixed.

extension GridViewController {
    private static let listedProblems: UInt64 = 5_000

    var isRepairVisible: Bool { shownSidePanel == .repair }

    @objc func toggleRepairPanel(_ sender: Any?) {
        showSidePanel(isRepairVisible ? nil : .repair)
    }

    func configureProblemsButton() {
        problemsButton.isBordered = false
        problemsButton.image = NSImage(systemSymbolName: "exclamationmark.triangle.fill", accessibilityDescription: nil)
        problemsButton.contentTintColor = .systemOrange
        problemsButton.imagePosition = .imageLeading
        problemsButton.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        problemsButton.target = self
        problemsButton.action = #selector(toggleRepairPanel(_:))
        problemsButton.toolTip = L("Find and fix broken rows (⌥⌘R)")
        problemsButton.isHidden = true
    }

    /// Looks for broken rows off the main thread, a moment after the last change.
    func scheduleProblemCheck(after delay: TimeInterval = 0.3) {
        problemCheck?.cancel()
        let check = DispatchWorkItem { [weak self] in self?.checkProblems() }
        problemCheck = check
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: check)
    }

    private func checkProblems() {
        problemGeneration += 1
        let generation = problemGeneration
        repairModel.isLoading = true
        let core = core
        DispatchQueue.global(qos: .userInitiated).async {
            let report = core.findProblems(limit: Self.listedProblems)
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.problemGeneration else { return }
                self.show(report)
            }
        }
    }

    private func show(_ report: RepairReport) {
        let firstCheck = repairModel.report == nil
        repairModel.isLoading = false
        repairModel.report = report
        problemRows = Dictionary(report.problems.map { (Int($0.row), $0.kind) }, uniquingKeysWith: { first, _ in first })
        let total = report.counts.reduce(0) { $0 + Int($1.count) }
        problemsButton.title = L("Problems: %@", Self.format(total))
        problemsButton.isHidden = total == 0
        redrawRowNumbers()
        // A file that opens with broken rows shows them right away.
        if firstCheck, total > 0, shownSidePanel == nil {
            showSidePanel(.repair)
        }
        reselectProblem()
    }

    func configureRepairActions() {
        repairModel.rowLabel = { [weak self] row in
            guard let self else { return "" }
            let row = Int(row)
            return row < self.headerOffset ? L("Header") : L("Row %@", Self.format(row - self.headerOffset + 1))
        }
        repairModel.columnTitle = { [weak self] column in
            self?.columnTitle(column) ?? ""
        }
        repairModel.onSelect = { [weak self] problem in
            self?.selectProblem(problem, moveCursor: true)
        }
        repairModel.onApply = { [weak self] row, fix in
            guard let self else { return }
            self.repairModel.pendingIndex = self.repairModel.report?.problems.firstIndex { $0.row == row }
            self.repairModel.selectedRow = nil
            self.document.applyFix(row: Int(row), fix: fix)
        }
        repairModel.onFixAll = { [weak self] kind in
            guard let self else { return }
            self.repairModel.pendingIndex = 0
            self.repairModel.selectedRow = nil
            self.document.fixAll(kind)
        }
    }

    /// The problem of the active row becomes the selected one in the panel.
    func repairCursorMoved() {
        guard isRepairVisible, let fileRow = activeFileRow, problemRows[fileRow] != nil,
              UInt64(fileRow) != repairModel.selectedRow,
              let problem = repairModel.report?.problems.first(where: { $0.row == UInt64(fileRow) }) else { return }
        selectProblem(problem, moveCursor: false)
    }

    /// After a check, keeps the selected problem, or after a fix moves on to the next one.
    private func reselectProblem() {
        guard let report = repairModel.report else { return }
        if let row = repairModel.selectedRow, let problem = report.problems.first(where: { $0.row == row }) {
            selectProblem(problem, moveCursor: false)
        } else if let index = repairModel.pendingIndex, !report.problems.isEmpty {
            selectProblem(report.problems[min(index, report.problems.count - 1)], moveCursor: true)
        } else {
            repairModel.selectedRow = nil
            repairModel.options = []
            repairCursorMoved()
        }
        repairModel.pendingIndex = nil
    }

    /// Shows the fixes of a problem, and with `moveCursor` its row in the table.
    private func selectProblem(_ problem: Problem, moveCursor: Bool) {
        repairModel.selectedRow = problem.row
        repairModel.rawText = (try? core.rawText(row: problem.row, maxChars: 600)) ?? ""
        repairModel.optionsGeneration += 1
        let generation = repairModel.optionsGeneration
        let core = core
        DispatchQueue.global(qos: .userInitiated).async {
            let options = (try? core.repairOptions(row: problem.row)) ?? []
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.repairModel.optionsGeneration else { return }
                self.repairModel.options = options
            }
        }
        if moveCursor, let visible = core.visibleIndex(row: problem.row) {
            selectCell(row: Int(visible), column: activeColumn)
        }
    }

    private func redrawRowNumbers() {
        let visible = tableView.rows(in: tableView.visibleRect)
        guard visible.length > 0, tableView.numberOfColumns > 0 else { return }
        tableView.reloadData(
            forRowIndexes: IndexSet(integersIn: visible.location..<NSMaxRange(visible)),
            columnIndexes: IndexSet(integer: 0)
        )
    }
}
