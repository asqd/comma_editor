import SwiftUI

/// State of the record card; the grid fills it with the active row and handles its actions.
@MainActor
final class RecordCardModel: ObservableObject {
    struct Field: Identifiable, Equatable {
        /// Visible row the value belongs to.
        let row: Int
        let column: Int
        let name: String
        let value: String
        var id: Int { column }
    }

    struct Edit {
        let row: Int
        let column: Int
        let value: String
    }

    @Published private(set) var row: Int?
    @Published private(set) var title = ""
    @Published private(set) var position = ""
    @Published private(set) var fields: [Field] = []
    @Published private(set) var activeColumn = 0
    @Published private(set) var canGoBack = false
    @Published private(set) var canGoForward = false
    @Published var query = ""
    @Published var hidesEmpty = false
    /// Text typed into a field and not yet written to the document.
    private(set) var pending: Edit?

    var onEdit: ((Edit) -> Void)?
    var onSelectColumn: ((Int, Int) -> Void)?
    var onPrevious: (() -> Void)?
    var onNext: (() -> Void)?

    func show(row: Int, title: String, position: String, fields: [Field], activeColumn: Int, canGoBack: Bool, canGoForward: Bool) {
        self.row = row
        self.title = title
        self.position = position
        if fields != self.fields {
            self.fields = fields
        }
        self.activeColumn = activeColumn
        self.canGoBack = canGoBack
        self.canGoForward = canGoForward
    }

    func clear() {
        row = nil
        fields = []
        pending = nil
    }

    var visibleFields: [Field] {
        let query = query.trimmingCharacters(in: .whitespaces)
        return fields.filter { field in
            (!hidesEmpty || !field.value.isEmpty)
                && (query.isEmpty
                    || field.name.localizedCaseInsensitiveContains(query)
                    || field.value.localizedCaseInsensitiveContains(query))
        }
    }

    func draftChanged(_ field: Field, to text: String) {
        if text != field.value {
            pending = Edit(row: field.row, column: field.column, value: text)
        } else if pending?.row == field.row && pending?.column == field.column {
            pending = nil
        }
    }

    /// Writes the text typed into a field, if any.
    func commitPending() {
        guard let edit = pending else { return }
        pending = nil
        onEdit?(edit)
    }
}

/// The active row as a form: one editable field per column, for tables too wide to read
/// across. Follows the cursor of the table.
struct RecordCardView: View {
    @ObservedObject var model: RecordCardModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            if model.row == nil {
                Text(L("No rows"))
                    .foregroundStyle(.secondary)
                    .padding(14)
                Spacer()
            } else {
                fieldList
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.title).font(.headline)
                    Text(model.position).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
                Spacer()
                ControlGroup {
                    Button { model.onPrevious?() } label: { Image(systemName: "chevron.up") }
                        .disabled(!model.canGoBack)
                        .help(L("Previous record (⌘[)"))
                    Button { model.onNext?() } label: { Image(systemName: "chevron.down") }
                        .disabled(!model.canGoForward)
                        .help(L("Next record (⌘])"))
                }
                .fixedSize()
            }
            HStack(spacing: 8) {
                TextField(L("Find a field"), text: $model.query)
                    .textFieldStyle(.roundedBorder)
                Toggle(L("Hide empty"), isOn: $model.hidesEmpty)
                    .toggleStyle(.checkbox)
                    .fixedSize()
            }
            .controlSize(.small)
        }
        .padding(14)
    }

    private var fieldList: some View {
        ScrollViewReader { scroller in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.visibleFields) { field in
                        RecordFieldRow(
                            field: field,
                            isActive: field.column == model.activeColumn,
                            onDraft: { model.draftChanged(field, to: $0) },
                            onCommit: { model.commitPending() },
                            onSelect: { model.onSelectColumn?(field.row, field.column) }
                        )
                        .id(field.column)
                    }
                }
                .padding(.vertical, 6)
                // A new row starts with fresh drafts.
                .id(model.row)
            }
            .onChange(of: model.activeColumn) { _, column in
                withAnimation { scroller.scrollTo(column) }
            }
        }
    }
}

private struct RecordFieldRow: View {
    let field: RecordCardModel.Field
    let isActive: Bool
    let onDraft: (String) -> Void
    let onCommit: () -> Void
    let onSelect: () -> Void

    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button(action: onSelect) {
                Text(field.name)
                    .font(.callout)
                    .foregroundStyle(isActive ? Color.accentColor : Color.secondary)
                    .lineLimit(2)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 110, alignment: .trailing)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(field.name)
            TextField(L("(empty)"), text: $draft, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...8)
                .focused($focused)
                .onSubmit(onCommit)
                .onExitCommand {
                    draft = field.value
                    focused = false
                }
                .padding(.vertical, 3)
                .padding(.horizontal, 5)
                .background(RoundedRectangle(cornerRadius: 4).fill(Color(nsColor: .textBackgroundColor)))
                .overlay(
                    RoundedRectangle(cornerRadius: 4)
                        .stroke(focused ? Color.accentColor : Color.secondary.opacity(0.2))
                )
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 3)
        .background(isActive ? Color.accentColor.opacity(0.08) : Color.clear)
        .onAppear { draft = field.value }
        .onChange(of: field.value) { _, value in draft = value }
        .onChange(of: draft) { _, text in onDraft(text) }
        .onChange(of: focused) { _, isFocused in
            if isFocused {
                onSelect()
            } else {
                onCommit()
            }
        }
    }
}
