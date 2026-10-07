import AppKit

/// Cell text with invisible characters made visible: spaces at either end, tabs, no-break
/// and zero-width spaces stand out as marks, line breaks show as ↵, and an empty value as ∅.
/// For display only; the marks never reach the data.
enum InvisibleText {
    static let edgeSpace = "·"
    static let tab = "→"
    static let noBreakSpace = "°"
    static let zeroWidth = "¦"
    static let lineBreak = "↵"
    static let empty = "∅"

    private static let noBreakSpaces: Set<Unicode.Scalar> = ["\u{00A0}", "\u{202F}", "\u{2007}"]
    private static let zeroWidths: Set<Unicode.Scalar> = ["\u{200B}", "\u{200C}", "\u{200D}", "\u{2060}", "\u{FEFF}"]

    static func render(_ value: String, wraps: Bool, font: NSFont) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = wraps ? .byWordWrapping : .byTruncatingTail
        let plain: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: paragraph,
        ]
        var quiet = plain
        quiet[.foregroundColor] = NSColor.tertiaryLabelColor
        var mark = plain
        mark[.foregroundColor] = NSColor.systemOrange
        mark[.backgroundColor] = NSColor.systemOrange.withAlphaComponent(0.15)

        guard !value.isEmpty else {
            return NSAttributedString(string: empty, attributes: quiet)
        }
        let scalars = Array(value.unicodeScalars)
        let leading = scalars.prefix { $0 == " " }.count
        let trailing = leading == scalars.count ? 0 : scalars.reversed().prefix { $0 == " " }.count

        let result = NSMutableAttributedString()
        var text = ""
        func flush() {
            if !text.isEmpty {
                result.append(NSAttributedString(string: text, attributes: plain))
                text = ""
            }
        }
        func append(_ string: String, _ attributes: [NSAttributedString.Key: Any]) {
            flush()
            result.append(NSAttributedString(string: string, attributes: attributes))
        }

        var index = 0
        while index < scalars.count {
            let scalar = scalars[index]
            switch scalar {
            case " " where index < leading || index >= scalars.count - trailing:
                append(edgeSpace, mark)
            case "\t":
                append(tab, mark)
            case "\r", "\n":
                if scalar == "\r", index + 1 < scalars.count, scalars[index + 1] == "\n" {
                    index += 1
                }
                append(wraps ? lineBreak + "\n" : " " + lineBreak + " ", quiet)
            case _ where noBreakSpaces.contains(scalar):
                append(noBreakSpace, mark)
            case _ where zeroWidths.contains(scalar):
                append(zeroWidth, mark)
            default:
                text.unicodeScalars.append(scalar)
            }
            index += 1
        }
        flush()
        return result
    }
}
