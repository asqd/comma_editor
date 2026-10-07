import SwiftUI

/// State of the encoding and delimiter sheet: the choices and how the file reads with them.
@MainActor
final class OpenOptionsModel: ObservableObject {
    @Published var encoding: TextEncoding? {
        didSet { refresh() }
    }
    @Published var delimiter: String? {
        didSet { refresh() }
    }
    @Published private(set) var preview: FilePreview?
    @Published private(set) var problem: String?
    /// Detection was not sure; the sheet opened by itself.
    let uncertain: Bool
    private let path: String
    var onApply: ((TextEncoding?, String?) -> Void)?
    var onCancel: (() -> Void)?

    init(path: String, encoding: TextEncoding?, delimiter: String?, uncertain: Bool) {
        self.path = path
        self.encoding = encoding
        self.delimiter = delimiter
        self.uncertain = uncertain
        refresh()
    }

    private func refresh() {
        do {
            preview = try previewFile(path: path, encoding: encoding, delimiter: delimiter, rows: 12)
            problem = nil
        } catch {
            preview = nil
            problem = CSVDocument.userFacingError(error).localizedDescription
        }
    }
}

/// File ▸ Check Encoding and Delimiter: the start of the file as it reads with the chosen
/// encoding and delimiter. Opens by itself when detection is not sure.
struct OpenOptionsView: View {
    @ObservedObject var model: OpenOptionsModel
    private static let shownColumns = 8

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(model.uncertain ? L("Check the Encoding and Delimiter") : L("Encoding and Delimiter"))
                .font(.headline)
            if model.uncertain {
                Text(L("CommaEditor is not sure how to read this file. Choose the encoding and delimiter with which the text reads correctly."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                GridRow {
                Text(L("Encoding:")).gridColumnAlignment(.trailing)
                Picker("", selection: $model.encoding) {
                    Text(detected(L("Detect Automatically"), model.encoding == nil ? model.preview.map { CSVDocument.encodingName($0.encoding) } : nil))
                        .tag(TextEncoding?.none)
                    Divider()
                    ForEach(CSVDocument.encodings, id: \.self) { encoding in
                        Text(CSVDocument.encodingName(encoding)).tag(TextEncoding?.some(encoding))
                    }
                }
                .labelsHidden()
                .fixedSize()
                }
                GridRow {
                Text(L("Delimiter:"))
                Picker("", selection: $model.delimiter) {
                    Text(detected(L("Detect Automatically"), model.delimiter == nil ? model.preview.map { CSVDocument.delimiterName($0.delimiter) } : nil))
                        .tag(String?.none)
                    Divider()
                    ForEach(CSVDocument.delimiters, id: \.self) { delimiter in
                        Text(CSVDocument.delimiterName(delimiter)).tag(String?.some(delimiter))
                    }
                }
                .labelsHidden()
                .fixedSize()
                }
            }
            previewTable
            HStack {
                Spacer()
                Button(model.uncertain ? L("Keep As Is") : L("Cancel")) { model.onCancel?() }
                    .keyboardShortcut(.cancelAction)
                Button(L("Reopen")) { model.onApply?(model.encoding, model.delimiter) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.preview == nil)
            }
        }
        .padding(20)
        .frame(width: 720)
    }

    private func detected(_ title: String, _ value: String?) -> String {
        value.map { "\(title) (\($0))" } ?? title
    }

    @ViewBuilder
    private var previewTable: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(L("Preview")).font(.subheadline.weight(.semibold))
            ScrollView([.vertical, .horizontal]) {
                VStack(alignment: .leading, spacing: 4) {
                if let preview = model.preview {
                    let columns = min(Int(preview.columnCount), Self.shownColumns)
                    Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 4) {
                        ForEach(Array(preview.rows.enumerated()), id: \.offset) { index, row in
                            GridRow {
                                ForEach(0..<columns, id: \.self) { column in
                                    Text(column < row.count ? GridViewController.singleLine(row[column]) : "")
                                        .fontWeight(index == 0 && preview.hasHeader ? .semibold : .regular)
                                        .lineLimit(1)
                                        .frame(maxWidth: 160, alignment: .leading)
                                }
                            }
                        }
                    }
                    .font(.callout)
                    .padding(8)
                    if preview.columnCount > UInt64(columns) {
                        Text(L("…and %@ more columns", GridViewController.format(Int(preview.columnCount) - columns)))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                    }
                } else if let problem = model.problem {
                    Text(problem).foregroundStyle(.secondary).padding(8)
                }
                }
                .frame(minWidth: 680, minHeight: 250, alignment: .topLeading)
            }
            .frame(height: 260)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.08)))
        }
    }
}
