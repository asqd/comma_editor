import Charts
import SwiftUI

/// State of the column profile panel; the grid fills it and handles its actions.
@MainActor
final class ColumnProfileModel: ObservableObject {
    @Published var columnName = ""
    @Published var profile: ColumnProfile?
    @Published var isLoading = false

    var onShowIssue: ((Issue) -> Void)?
    var onTrim: (() -> Void)?
    var onFilterValue: ((String) -> Void)?
}

/// Type, statistics, distribution, problems and frequent values of the active column.
struct ColumnProfileView: View {
    @ObservedObject var model: ColumnProfileModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                if let profile = model.profile {
                    statistics(profile)
                    if !profile.histogram.isEmpty {
                        histogram(profile)
                    }
                    issues(profile)
                    if !profile.topValues.isEmpty {
                        topValues(profile)
                    }
                } else if model.isLoading {
                    ProgressView().controlSize(.small)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(model.columnName)
                .font(.headline)
                .lineLimit(2)
            if let kind = model.profile?.kind {
                Text(Self.kindName(kind))
                    .font(.caption)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.accentColor.opacity(0.15)))
                    .foregroundStyle(Color.accentColor)
            }
        }
    }

    private func statistics(_ profile: ColumnProfile) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 5) {
            row(L("Values"), number(profile.total))
            row(L("Empty"), number(profile.empty) + percent(profile.empty, of: profile.total))
            row(L("Unique"), number(profile.unique))
            switch profile.kind {
            case .integer, .decimal:
                row(L("Minimum"), profile.min ?? "—")
                row(L("Maximum"), profile.max ?? "—")
                row(L("Average"), profile.mean.map(Self.decimal) ?? "—")
                row(L("Median"), profile.median.map(Self.decimal) ?? "—")
            case .date:
                row(L("Earliest"), profile.min ?? "—")
                row(L("Latest"), profile.max ?? "—")
            default:
                row(L("Length"), profile.minLength == profile.maxLength
                    ? number(profile.minLength)
                    : "\(number(profile.minLength))–\(number(profile.maxLength))")
            }
        }
        .font(.callout)
    }

    private func row(_ label: String, _ value: String) -> some View {
        // VoiceOver reads each value with its name.
        GridRow {
            Text(label).foregroundStyle(.secondary).accessibilityHidden(true)
            Text(value).monospacedDigit().textSelection(.enabled)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(label + ": " + value)
        }
    }

    private func histogram(_ profile: ColumnProfile) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(L("Distribution")).font(.subheadline.weight(.semibold))
            Chart(Array(profile.histogram.enumerated()), id: \.offset) { bin in
                BarMark(x: .value(L("Range"), bin.offset + 1), y: .value(L("Rows"), bin.element))
                    .foregroundStyle(Color.accentColor.opacity(0.75))
            }
            .chartXAxis(.hidden)
            .chartYAxis(.hidden)
            .frame(height: 64)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(L("Distribution from %@ to %@", profile.min ?? "", profile.max ?? ""))
            HStack {
                Text(profile.min ?? "")
                Spacer()
                Text(profile.max ?? "")
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func issues(_ profile: ColumnProfile) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L("Problems")).font(.subheadline.weight(.semibold))
            if profile.mismatched == 0 && profile.padded == 0 {
                Label(L("No problems found"), systemImage: "checkmark.circle")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            }
            if profile.mismatched > 0 {
                issueRow(
                    L("Not %@: %@", Self.kindName(profile.kind).lowercased(), number(profile.mismatched)),
                    actions: [(L("Show"), { model.onShowIssue?(.mismatch) })]
                )
            }
            if profile.padded > 0 {
                issueRow(
                    L("Spaces around the value: %@", number(profile.padded)),
                    actions: [
                        (L("Show"), { model.onShowIssue?(.padding) }),
                        (L("Remove"), { model.onTrim?() }),
                    ]
                )
            }
        }
    }

    private func issueRow(_ text: String, actions: [(String, () -> Void)]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(text, systemImage: "exclamationmark.triangle.fill")
                .symbolRenderingMode(.multicolor)
                .font(.callout)
            HStack(spacing: 6) {
                ForEach(actions.indices, id: \.self) { index in
                    Button(actions[index].0, action: actions[index].1)
                        .controlSize(.small)
                }
            }
            .padding(.leading, 22)
        }
    }

    private func topValues(_ profile: ColumnProfile) -> some View {
        let largest = Double(profile.topValues.first?.count ?? 1)
        return VStack(alignment: .leading, spacing: 4) {
            Text(L("Frequent values")).font(.subheadline.weight(.semibold))
            ForEach(profile.topValues, id: \.value) { item in
                Button {
                    model.onFilterValue?(item.value)
                } label: {
                    HStack {
                        Text(item.value.isEmpty ? L("(empty)") : item.value.replacingOccurrences(of: "\n", with: " ↵ "))
                            .lineLimit(1)
                        Spacer(minLength: 8)
                        Text(number(item.count)).monospacedDigit().foregroundStyle(.secondary)
                    }
                    .font(.callout)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(alignment: .leading) {
                        GeometryReader { geometry in
                            RoundedRectangle(cornerRadius: 4)
                                .fill(Color.accentColor.opacity(0.12))
                                .frame(width: geometry.size.width * Double(item.count) / largest)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(L("Show only rows with this value"))
            }
        }
    }

    private func number(_ value: UInt64) -> String {
        GridViewController.format(Int(value))
    }

    private func percent(_ part: UInt64, of total: UInt64) -> String {
        guard total > 0, part > 0 else { return "" }
        return " (" + (Double(part) / Double(total)).formatted(.percent.precision(.fractionLength(0...1))) + ")"
    }

    private static func decimal(_ value: Double) -> String {
        GridViewController.format(value)
    }

    static func kindName(_ kind: ValueKind) -> String {
        switch kind {
        case .empty: L("Empty column")
        case .integer: L("Whole numbers")
        case .decimal: L("Numbers")
        case .date: L("Dates")
        case .boolean: L("Yes or no")
        case .text: L("Text")
        }
    }
}
