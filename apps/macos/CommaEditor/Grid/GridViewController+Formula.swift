import AppKit
import SwiftUI

// Data ▸ Computed Column: a new column, or new values for the active one, from a formula.

extension GridViewController {
    private static let previewRows: UInt64 = 8

    @objc func computedColumn(_ sender: Any?) {
        guard columnCount > 0 else { return }
        let column = activeColumn
        let model = FormulaModel(
            columns: (0..<columnCount).map(columnTitle),
            activeColumnName: columnTitle(column)
        )
        let controller = NSHostingController(rootView: FormulaView(model: model))
        let core = core
        let firstRow = headerOffset
        model.evaluate = { formula in
            do {
                let outcomes = try core.previewFormula(formula: formula, start: 0, count: Self.previewRows)
                return .success(outcomes.enumerated().map { index, outcome in
                    let fileRow = core.visibleRecord(visibleRow: UInt64(index)).map { Int($0) } ?? index + firstRow
                    return FormulaModel.PreviewRow(
                        id: index,
                        label: L("Row %@", Self.format(fileRow - firstRow + 1)),
                        value: outcome.value ?? outcome.problem.map(FormulaHelp.describe) ?? "",
                        isError: outcome.problem != nil
                    )
                })
            } catch CoreError.Formula(let problem) {
                return .failure(FormulaModel.FormulaFailure(message: FormulaHelp.describe(problem)))
            } catch {
                return .failure(FormulaModel.FormulaFailure(message: error.localizedDescription))
            }
        }
        model.onCancel = { [weak self, weak controller] in
            guard let controller else { return }
            self?.dismiss(controller)
        }
        model.onApply = { [weak self, weak controller, weak model] in
            guard let self, let controller, let model else { return }
            self.dismiss(controller)
            self.applyFormula(model.formula, target: model.target, name: model.name, column: column)
        }
        presentAsSheet(controller)
    }

    private func applyFormula(_ formula: String, target: FormulaModel.Target, name: String, column: Int) {
        let row = activeRow ?? 0
        switch target {
        case .newColumn:
            document.addFormulaColumn(formula, name: name.trimmingCharacters(in: .whitespaces), at: column + 1) { [weak self] report in
                self?.selectCell(row: row, column: column + 1)
                self?.showFormulaErrors(report)
            }
        case .activeColumn:
            document.fillColumn(column, formula: formula) { [weak self] report in
                self?.showFormulaErrors(report)
            }
        }
    }

    private func showFormulaErrors(_ report: FormulaReport) {
        guard report.errors > 0, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("The formula failed in %@ of %@ rows.", Self.format(Int(report.errors)), Self.format(Int(report.rows)))
        var details = L("These cells were left empty. Undo with ⌘Z.")
        if let fileRow = report.firstErrorRow, let problem = report.firstError {
            details = L("First: row %@, %@.", Self.format(Int(fileRow) - headerOffset + 1), FormulaHelp.describe(problem)) + " " + details
        }
        alert.informativeText = details
        alert.beginSheetModal(for: window)
    }
}
