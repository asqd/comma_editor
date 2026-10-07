import SwiftUI

/// State of the repair panel; the grid fills it and handles its actions.
@MainActor
final class RepairModel: ObservableObject {
    @Published var report: RepairReport?
    @Published var isLoading = false
    /// File row of the problem whose fixes are shown.
    @Published var selectedRow: UInt64?
    @Published var rawText = ""
    @Published var options: [RepairOption] = []
    /// Counter that drops stale fix previews.
    var optionsGeneration = 0
    /// Index in the list to select after a fix, when the fixed row is gone from it.
    var pendingIndex: Int?

    var rowLabel: (UInt64) -> String = { L("Row %@", String($0 + 1)) }
    var columnTitle: (Int) -> String = { String($0 + 1) }
    var onSelect: ((Problem) -> Void)?
    var onApply: ((UInt64, RepairFix) -> Void)?
    var onFixAll: ((ProblemKind) -> Void)?
    /// A garbled value of the first rows and how it reads fixed.
    @Published var garbledExample: (garbled: String, fixed: String)?
    var onFixGarbled: (() -> Void)?
}

/// Broken rows of the file grouped by kind, and fixes with a preview for the selected one.
struct RepairView: View {
    @ObservedObject var model: RepairModel

    var body: some View {
        ScrollViewReader { scroller in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    Text(L("File Repair")).font(.headline)
                    if let report = model.report {
                        content(report)
                    } else if model.isLoading {
                        ProgressView().controlSize(.small)
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: model.selectedRow) { _, row in
                guard let row else { return }
                // After the list has laid out the selected row's fixes.
                DispatchQueue.main.async {
                    withAnimation { scroller.scrollTo(row, anchor: .top) }
                }
            }
        }
    }

    @ViewBuilder
    private func content(_ report: RepairReport) -> some View {
        if report.garbledSample > 0 {
            garbledGroup(count: report.garbledSample)
        }
        if report.counts.isEmpty {
            if report.garbledSample == 0 {
                Label(L("No problems found"), systemImage: "checkmark.circle")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            }
        } else {
            Text(L("Most rows have %@ fields.", number(report.expectedFields)))
                .font(.callout)
                .foregroundStyle(.secondary)
            ForEach(report.counts, id: \.kind) { count in
                group(count)
            }
            Divider()
            Text(L("Select a row to see how to fix it."))
                .font(.caption)
                .foregroundStyle(.secondary)
            LazyVStack(alignment: .leading, spacing: 2) {
                ForEach(report.problems, id: \.row) { problem in
                    problemRow(problem, expected: report.expectedFields)
                }
            }
            .padding(.horizontal, -8)
            let total = report.counts.reduce(0) { $0 + $1.count }
            if total > UInt64(report.problems.count) {
                Text(L("Showing the first %@ problems.", number(UInt64(report.problems.count))))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func garbledGroup(count: UInt64) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(L("Garbled text: %@", number(count)), systemImage: "exclamationmark.triangle.fill")
                .symbolRenderingMode(.multicolor)
                .font(.callout.weight(.medium))
            Text(L("Some values look like text read in the wrong encoding and saved that way. Counted in the first rows."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let example = model.garbledExample {
                Text("\(example.garbled) → \(example.fixed)")
                    .font(.system(.caption, design: .monospaced))
                    .lineLimit(2)
                    .textSelection(.enabled)
            }
            Button(L("Fix in the Whole File")) { model.onFixGarbled?() }
                .controlSize(.small)
        }
    }

    private func group(_ count: ProblemCount) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(Self.kindTitle(count.kind) + ": " + number(count.count), systemImage: "exclamationmark.triangle.fill")
                .symbolRenderingMode(.multicolor)
                .font(.callout.weight(.medium))
            Text(Self.kindDescription(count.kind))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button(L("Fix All")) { model.onFixAll?(count.kind) }
                .controlSize(.small)
                .help(Self.fixAllHelp(count.kind))
        }
    }

    private func problemRow(_ problem: Problem, expected: UInt64) -> some View {
        let selected = model.selectedRow == problem.row
        return VStack(alignment: .leading, spacing: 8) {
            Button {
                model.onSelect?(problem)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(model.rowLabel(problem.row)).font(.callout.weight(.medium))
                        Text(Self.kindTitle(problem.kind)).font(.caption).foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                    }
                    Text(detail(problem, expected: expected))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if selected {
                Text(model.rawText)
                    .font(.system(.caption, design: .monospaced))
                    .lineLimit(5)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(6)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Color.secondary.opacity(0.1)))
                ForEach(Array(model.options.enumerated()), id: \.offset) { index, option in
                    optionView(option, row: problem.row, recommended: index == 0)
                }
            }
        }
        .padding(8)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? Color.accentColor.opacity(0.1) : Color.clear))
        .id(problem.row)
    }

    private func optionView(_ option: RepairOption, row: UInt64, recommended: Bool) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            if recommended {
                Text(L("Recommended"))
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(Color.accentColor)
            }
            Button(fixTitle(option.fix)) { model.onApply?(row, option.fix) }
                .controlSize(.small)
            Text(preview(option))
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(7)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Text

    private func detail(_ problem: Problem, expected: UInt64) -> String {
        if problem.lines > 1 {
            return L("Takes %@ lines of the file", number(problem.lines))
        }
        return L("Fields: %@ of %@", number(problem.fields), number(expected))
    }

    private func preview(_ option: RepairOption) -> String {
        if option.rowCount == 0 {
            return L("The row is deleted.")
        }
        var lines = option.rows.map { row in
            row.map { $0.isEmpty ? "∅" : GridViewController.singleLine($0) }.joined(separator: " │ ")
        }
        if option.rowCount > UInt64(option.rows.count) {
            lines.append(L("…and %@ more rows", number(option.rowCount - UInt64(option.rows.count))))
        }
        return lines.joined(separator: "\n")
    }

    private func fixTitle(_ fix: RepairFix) -> String {
        switch fix {
        case .quoteAsText: L("Keep the quote as a character")
        case .removeQuote: L("Remove the quote")
        case .padFields: L("Add empty fields")
        case .joinNext: L("Join with the next row")
        case .mergeFields(let column): L("Merge extra fields into “%@”", model.columnTitle(Int(column)))
        case .dropEmptyFields: L("Remove empty fields at the end")
        case .deleteRow: L("Delete the row")
        }
    }

    private func number(_ value: UInt64) -> String {
        GridViewController.format(Int(value))
    }

    static func kindTitle(_ kind: ProblemKind) -> String {
        switch kind {
        case .unclosedQuote: L("Unclosed quote")
        case .strayQuote: L("Stray quote")
        case .missingFields: L("Missing fields")
        case .extraFields: L("Extra fields")
        }
    }

    private static func kindDescription(_ kind: ProblemKind) -> String {
        switch kind {
        case .unclosedQuote:
            L("A quote opens a value and never closes, so everything up to the end of the file became one cell.")
        case .strayQuote:
            L("A quote merged several fields or rows into one value.")
        case .missingFields:
            L("Fewer fields than in other rows: a line break inside a value without quotes, or a blank line.")
        case .extraFields:
            L("More fields than in other rows: usually a delimiter inside a value without quotes.")
        }
    }

    private static func fixAllHelp(_ kind: ProblemKind) -> String {
        switch kind {
        case .unclosedQuote, .strayQuote:
            L("Keeps each quote as an ordinary character")
        case .missingFields:
            L("Joins rows split by a line break, deletes blank lines, adds empty fields to the rest")
        case .extraFields:
            L("Removes empty fields at the end or merges extra fields into the column where they fit")
        }
    }
}
