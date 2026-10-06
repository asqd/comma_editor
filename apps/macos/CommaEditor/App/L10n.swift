import Foundation

/// Localized string; the English text is the key.
func L(_ key: String) -> String {
    NSLocalizedString(key, comment: "")
}

/// Localized format string filled with `arguments`.
func L(_ key: String, _ arguments: CVarArg...) -> String {
    String(format: NSLocalizedString(key, comment: ""), arguments: arguments)
}
