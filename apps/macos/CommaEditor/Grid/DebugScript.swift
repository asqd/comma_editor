#if DEBUG
import AppKit

/// Debug builds only: applies UI steps from `COMMA_DEBUG_SCRIPT` to the first window, so the
/// sort, filter and find UI can be checked from the command line. Steps are separated by `;`:
/// `sort=<column>:a|d`, `filter=<column|*>:<op>:<value>`, `find=<text>`, `replace`, `exportexcel`.
/// Example: `open --env COMMA_DEBUG_SCRIPT='sort=2:d;find=lon' -a CommaEditor file.csv`
@MainActor
enum DebugScript {
    private static var done = false

    static func run(on grid: GridViewController) {
        guard !done, let script = ProcessInfo.processInfo.environment["COMMA_DEBUG_SCRIPT"] else { return }
        done = true
        let ops: [String: FilterOp] = [
            "contains": .contains, "equals": .equals, "notequals": .notEquals,
            "greater": .greaterThan, "less": .lessThan, "empty": .isEmpty, "regex": .matches,
        ]
        for step in script.split(separator: ";") {
            let (name, argument) = step.split(separator: "=", maxSplits: 1).map(String.init).pair
            let parts = argument.split(separator: ":", maxSplits: 2).map(String.init)
            switch name {
            case "sort" where parts.count == 2:
                grid.setSort([SortKey(column: UInt64(parts[0]) ?? 0, ascending: parts[1] != "d")])
            case "filter" where parts.count == 3:
                grid.addFilter(FilterRule(
                    column: UInt64(parts[0]),
                    op: ops[parts[1]] ?? .contains,
                    value: parts[2],
                    caseSensitive: false
                ))
            case "find":
                grid.showFindBar(nil)
                grid.findBar.searchField.stringValue = argument
                grid.findBarQueryDidChange(grid.findBar)
            case "replace":
                grid.showFindAndReplaceBar(nil)
            case "exportexcel":
                grid.document.exportForExcel(nil)
            default:
                NSLog("COMMA_DEBUG_SCRIPT: unknown step \(step)")
            }
        }
    }
}

private extension Array where Element == String {
    var pair: (String, String) { (first ?? "", count > 1 ? self[1] : "") }
}
#endif
