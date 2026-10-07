import SwiftUI

/// State of the computed column sheet; the grid computes the preview and applies the formula.
@MainActor
final class FormulaModel: ObservableObject {
    enum Target: Hashable {
        case newColumn
        case activeColumn
    }

    struct PreviewRow: Identifiable {
        let id: Int
        let label: String
        let value: String
        let isError: Bool
    }

    @Published var formula = "" {
        didSet { refresh() }
    }
    @Published var name = L("Result")
    @Published var target = Target.newColumn
    @Published private(set) var preview: [PreviewRow] = []
    /// Why the formula does not compile, if it does not.
    @Published private(set) var problem: String?

    let columns: [String]
    let activeColumnName: String
    /// Computes the preview: rows, or the reason the formula does not compile.
    var evaluate: ((String) -> Result<[PreviewRow], FormulaFailure>)?
    var onApply: (() -> Void)?
    var onCancel: (() -> Void)?

    struct FormulaFailure: Error {
        let message: String
    }

    init(columns: [String], activeColumnName: String) {
        self.columns = columns
        self.activeColumnName = activeColumnName
    }

    var canApply: Bool {
        !formula.trimmingCharacters(in: .whitespaces).isEmpty && problem == nil
            && (target == .activeColumn || !name.trimmingCharacters(in: .whitespaces).isEmpty)
    }

    func insert(_ text: String) {
        let separator = formula.isEmpty || formula.hasSuffix(" ") || formula.hasSuffix("(") ? "" : " "
        formula += separator + text
    }

    private func refresh() {
        guard !formula.trimmingCharacters(in: .whitespaces).isEmpty, let evaluate else {
            preview = []
            problem = nil
            return
        }
        switch evaluate(formula) {
        case .success(let rows):
            preview = rows
            problem = nil
        case .failure(let failure):
            preview = []
            problem = failure.message
        }
    }
}

/// Data ▸ Computed Column: a formula over the columns of each row, with a live preview.
struct FormulaView: View {
    @ObservedObject var model: FormulaModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("Computed Column")).font(.headline)
            formulaField
            Picker("", selection: $model.target) {
                Text(L("New column")).tag(FormulaModel.Target.newColumn)
                Text(L("Replace “%@”", model.activeColumnName)).tag(FormulaModel.Target.activeColumn)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            if model.target == .newColumn {
                HStack {
                    Text(L("Name:"))
                    TextField("", text: $model.name)
                        .textFieldStyle(.roundedBorder)
                }
                Text(L("The column is added to the right of “%@”.", model.activeColumnName))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text(L("Values of the shown rows are replaced. The formula can use the column itself."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            previewList
            HStack {
                Spacer()
                Button(L("Cancel")) { model.onCancel?() }
                    .keyboardShortcut(.cancelAction)
                Button(model.target == .newColumn ? L("Add Column") : L("Replace Values")) { model.onApply?() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canApply)
            }
        }
        .padding(20)
        .frame(width: 560)
    }

    private var formulaField: some View {
        VStack(alignment: .leading, spacing: 6) {
            TextField("{price} * {qty}", text: $model.formula, axis: .vertical)
                .font(.system(.body, design: .monospaced))
                .textFieldStyle(.roundedBorder)
                .lineLimit(2...5)
            HStack(spacing: 8) {
                Menu(L("Column")) {
                    ForEach(Array(model.columns.enumerated()), id: \.offset) { _, name in
                        Button(name) { model.insert("{\(name)}") }
                    }
                }
                .fixedSize()
                Menu(L("Function")) {
                    ForEach(FormulaHelp.functions, id: \.name) { function in
                        Button("\(function.signature) — \(function.summary)") {
                            model.insert(function.name + "(")
                        }
                    }
                }
                .fixedSize()
                Spacer()
            }
            .controlSize(.small)
            if let problem = model.problem {
                Label(problem, systemImage: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
                    .font(.callout)
            } else {
                Text(L("Columns go in braces: {price}. Text goes in quotes: \"kg\". & joins text; +, -, *, / and % count; =, <>, <, > compare; AND, OR, NOT combine."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var previewList: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(L("Preview")).font(.subheadline.weight(.semibold))
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 3) {
                ForEach(model.preview) { row in
                    GridRow {
                        Text(row.label).foregroundStyle(.secondary).monospacedDigit()
                        Text(row.value.isEmpty ? "∅" : row.value)
                            .foregroundStyle(row.isError ? Color.orange : (row.value.isEmpty ? Color.secondary : Color.primary))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .textSelection(.enabled)
                    }
                }
            }
            .font(.callout)
            .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
            .padding(8)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.08)))
        }
    }
}

/// Functions of the formula language as people read them.
enum FormulaHelp {
    struct Function {
        let name: String
        let signature: String
        let summary: String
    }

    static let functions: [Function] = [
        Function(name: "IF", signature: L("IF(condition, then, else)"), summary: L("one of two values")),
        Function(name: "ROUND", signature: L("ROUND(number, digits)"), summary: L("round")),
        Function(name: "ABS", signature: "ABS(number)", summary: L("without sign")),
        Function(name: "MIN", signature: "MIN(a, b, …)", summary: L("smallest")),
        Function(name: "MAX", signature: "MAX(a, b, …)", summary: L("largest")),
        Function(name: "SUM", signature: "SUM(a, b, …)", summary: L("sum")),
        Function(name: "LEN", signature: L("LEN(text)"), summary: L("length")),
        Function(name: "UPPER", signature: L("UPPER(text)"), summary: L("upper case")),
        Function(name: "LOWER", signature: L("LOWER(text)"), summary: L("lower case")),
        Function(name: "TRIM", signature: L("TRIM(text)"), summary: L("without spaces at the ends")),
        Function(name: "LEFT", signature: L("LEFT(text, count)"), summary: L("first characters")),
        Function(name: "RIGHT", signature: L("RIGHT(text, count)"), summary: L("last characters")),
        Function(name: "MID", signature: L("MID(text, start, count)"), summary: L("characters from a position")),
        Function(name: "REPLACE", signature: L("REPLACE(text, find, with)"), summary: L("replace text")),
        Function(name: "CONTAINS", signature: L("CONTAINS(text, part)"), summary: L("whether the text contains the part")),
        Function(name: "CONCAT", signature: "CONCAT(a, b, …)", summary: L("join")),
        Function(name: "COALESCE", signature: "COALESCE(a, b, …)", summary: L("first value that is not empty")),
        Function(name: "ISEMPTY", signature: L("ISEMPTY(value)"), summary: L("whether the value is empty")),
        Function(name: "YEAR", signature: L("YEAR(date)"), summary: L("year")),
        Function(name: "MONTH", signature: L("MONTH(date)"), summary: L("month")),
        Function(name: "DAY", signature: L("DAY(date)"), summary: L("day of the month")),
        Function(name: "DAYS", signature: L("DAYS(end, start)"), summary: L("days between dates")),
    ]

    /// A formula problem in words.
    static func describe(_ problem: FormulaProblem) -> String {
        let detail = problem.detail
        switch problem.kind {
        case .unexpectedEnd: return L("The formula is incomplete.")
        case .unexpectedText:
            return detail.isEmpty
                ? L("Unexpected text at position %lld.", Int64(problem.position) + 1)
                : L("Unexpected “%@” at position %lld.", detail, Int64(problem.position) + 1)
        case .unclosedText: return L("A quoted text is not closed.")
        case .unclosedColumn: return L("A column name is not closed with }.")
        case .unknownColumn: return L("There is no column “%@”.", detail)
        case .unknownFunction: return L("There is no function %@.", detail)
        case .wrongArgumentCount: return L("Wrong number of arguments for %@.", detail)
        case .notANumber: return L("“%@” is not a number", detail)
        case .notADate: return L("“%@” is not a date", detail)
        case .divisionByZero: return L("division by zero")
        }
    }
}
