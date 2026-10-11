import AppKit
import UniformTypeIdentifiers

/// Encoding, delimiter and line endings, shown under the Save and Export panels.
final class SaveFormatAccessoryView: NSView {
    /// Called when the user changes the format.
    var onChange: ((SaveFormat) -> Void)?

    private let encodingPopUp = NSPopUpButton()
    private let delimiterPopUp = NSPopUpButton()
    private let lineEndingPopUp = NSPopUpButton()

    private static let encodings: [(TextEncoding, Bool, String)] = [
        (.utf8, false, "UTF-8"),
        (.utf8, true, L("UTF-8 with BOM")),
        (.utf16Le, true, "UTF-16LE"),
        (.utf16Be, true, "UTF-16BE"),
        (.windows1251, false, L("Windows-1251 (Cyrillic)")),
        (.koi8R, false, "KOI8-R"),
        (.ibm866, false, L("CP866 (DOS Cyrillic)")),
        (.koi8U, false, "KOI8-U"),
        (.windows1252, false, L("Windows-1252 (Western)")),
        (.windows1250, false, L("Windows-1250 (Central European)")),
        (.windows1253, false, L("Windows-1253 (Greek)")),
        (.windows1254, false, L("Windows-1254 (Turkish)")),
        (.windows1257, false, L("Windows-1257 (Baltic)")),
    ]
    private var delimiters: [(String, String)] = [
        (",", L("Comma (,)")),
        (";", L("Semicolon (;)")),
        ("\t", L("Tab")),
        ("|", L("Vertical Bar (|)")),
    ]
    private static let lineEndings: [(LineEnding, String)] = [
        (.lf, L("LF — macOS, Linux")),
        (.crLf, L("CRLF — Windows")),
        (.cr, L("CR — classic Mac OS")),
    ]

    var format: SaveFormat {
        get {
            let encoding = Self.encodings[encodingPopUp.indexOfSelectedItem]
            return SaveFormat(
                encoding: encoding.0,
                bom: encoding.1,
                delimiter: delimiters[delimiterPopUp.indexOfSelectedItem].0,
                lineEnding: Self.lineEndings[lineEndingPopUp.indexOfSelectedItem].0
            )
        }
        set {
            let encoding = Self.encodings.firstIndex { $0.0 == newValue.encoding && ($0.1 == newValue.bom || newValue.encoding.isUTF16) }
            encodingPopUp.selectItem(at: encoding ?? 0)
            if !delimiters.contains(where: { $0.0 == newValue.delimiter }) {
                delimiters.append((newValue.delimiter, L("Other: “%@”", newValue.delimiter)))
                delimiterPopUp.addItem(withTitle: delimiters.last!.1)
            }
            delimiterPopUp.selectItem(at: delimiters.firstIndex { $0.0 == newValue.delimiter } ?? 0)
            lineEndingPopUp.selectItem(at: Self.lineEndings.firstIndex { $0.0 == newValue.lineEnding } ?? 0)
        }
    }

    init(format: SaveFormat) {
        super.init(frame: .zero)
        encodingPopUp.addItems(withTitles: Self.encodings.map(\.2))
        delimiterPopUp.addItems(withTitles: delimiters.map(\.1))
        lineEndingPopUp.addItems(withTitles: Self.lineEndings.map(\.1))
        for popUp in [encodingPopUp, delimiterPopUp, lineEndingPopUp] {
            popUp.target = self
            popUp.action = #selector(changed(_:))
        }
        let excel = NSButton(title: L("For Excel"), target: self, action: #selector(useExcelPreset(_:)))
        excel.toolTip = L("UTF-8 with BOM, CRLF and the list separator Excel uses with your region settings")

        let label = { (text: String) -> NSTextField in
            let field = NSTextField(labelWithString: text)
            field.alignment = .right
            return field
        }
        let grid = NSGridView(views: [
            [label(L("Encoding:")), encodingPopUp],
            [label(L("Delimiter:")), delimiterPopUp],
            [label(L("Line Endings:")), lineEndingPopUp],
            [NSGridCell.emptyContentView, excel],
        ])
        grid.rowSpacing = 8
        grid.columnSpacing = 8
        grid.column(at: 0).xPlacement = .trailing
        grid.translatesAutoresizingMaskIntoConstraints = false
        addSubview(grid)
        NSLayoutConstraint.activate([
            grid.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            grid.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            grid.centerXAnchor.constraint(equalTo: centerXAnchor),
            grid.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 20),
        ])
        self.format = format
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    /// UTF-8 with BOM so Excel detects the encoding, CRLF, and the delimiter Excel expects:
    /// in regions with a decimal comma it splits CSV files by semicolons.
    func applyExcelPreset() {
        let delimiter = Locale.current.decimalSeparator == "," ? ";" : ","
        format = SaveFormat(encoding: .utf8, bom: true, delimiter: delimiter, lineEnding: .crLf)
        onChange?(format)
    }

    static func contentType(for format: SaveFormat) -> UTType {
        format.delimiter == "\t" ? .tabSeparatedText : .commaSeparatedText
    }

    @objc private func changed(_ sender: Any?) {
        onChange?(format)
    }

    @objc private func useExcelPreset(_ sender: Any?) {
        applyExcelPreset()
    }
}

extension TextEncoding {
    var isUTF16: Bool { self == .utf16Le || self == .utf16Be }
}
