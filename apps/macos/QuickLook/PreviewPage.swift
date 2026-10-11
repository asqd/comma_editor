import Foundation

/// The HTML page of a preview: a table with a sticky header and row numbers, in the system
/// font, in light and dark appearance.
enum PreviewPage {
    private static let maxColumns = 60
    private static let maxCharacters = 300

    static func html(for preview: FilePreview, limit: Int) -> String {
        let columns = min(Int(preview.columnCount), maxColumns)
        var body = ""
        var dataRows = preview.rows[...]
        if preview.hasHeader, let header = dataRows.first {
            dataRows = dataRows.dropFirst()
            body += "<thead><tr><th class=\"n\"></th>"
            for column in 0..<columns {
                body += "<th>\(cell(header, column))</th>"
            }
            body += "</tr></thead>"
        }
        body += "<tbody>"
        for (index, row) in dataRows.enumerated() {
            body += "<tr><td class=\"n\">\(index + 1)</td>"
            for column in 0..<columns {
                body += "<td>\(cell(row, column))</td>"
            }
            body += "</tr>"
        }
        body += "</tbody>"

        var notes = [
            encodingName(preview.encoding),
            String(format: localized("Delimiter: %@"), delimiterName(preview.delimiter)),
        ]
        if preview.rows.count >= limit {
            notes.append(String(format: localized("first %ld rows"), dataRows.count))
        }
        if Int(preview.columnCount) > columns {
            notes.append(String(format: localized("first %ld columns"), columns))
        }
        return """
        <!doctype html><html><head><meta charset="utf-8"><style>\(style)</style></head>
        <body><table>\(body)</table><footer>\(notes.map(escape).joined(separator: " · "))</footer></body></html>
        """
    }

    private static func cell(_ row: [String], _ column: Int) -> String {
        guard column < row.count else { return "" }
        var value = row[column]
        if value.count > maxCharacters {
            value = String(value.prefix(maxCharacters)) + "…"
        }
        return escape(value).replacingOccurrences(of: "\n", with: "<br>")
    }

    private static func escape(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
    }

    private static func localized(_ key: String) -> String {
        Bundle.main.localizedString(forKey: key, value: key, table: nil)
    }

    private static func encodingName(_ encoding: TextEncoding) -> String {
        switch encoding {
        case .utf8: "UTF-8"
        case .utf16Le: "UTF-16LE"
        case .utf16Be: "UTF-16BE"
        case .windows1251: "Windows-1251"
        case .koi8R: "KOI8-R"
        case .windows1252: "Windows-1252"
        case .windows1250: "Windows-1250"
        case .windows1253: "Windows-1253"
        case .windows1254: "Windows-1254"
        case .windows1257: "Windows-1257"
        case .koi8U: "KOI8-U"
        case .ibm866: "CP866"
        }
    }

    private static func delimiterName(_ delimiter: String) -> String {
        switch delimiter {
        case "\t": localized("tab")
        case " ": localized("space")
        default: delimiter
        }
    }

    private static let style = """
    :root { color-scheme: light dark; --line: #e2e2e2; --stripe: #f6f6f6; --head: #fafafa; --quiet: #8a8a8a; }
    @media (prefers-color-scheme: dark) {
      :root { --line: #3a3a3a; --stripe: #262626; --head: #2c2c2c; --quiet: #8e8e8e; }
    }
    body { margin: 0; font: 12px -apple-system, system-ui, sans-serif; }
    table { border-collapse: collapse; }
    th, td { border: 1px solid var(--line); padding: 3px 8px; text-align: left; vertical-align: top;
             max-width: 320px; overflow: hidden; text-overflow: ellipsis; }
    th { position: sticky; top: 0; background: var(--head); font-weight: 600; white-space: nowrap; }
    tbody tr:nth-child(even) td { background: var(--stripe); }
    td.n, th.n { color: var(--quiet); text-align: right; font-variant-numeric: tabular-nums; }
    footer { padding: 8px 10px; color: var(--quiet); }
    """
}
