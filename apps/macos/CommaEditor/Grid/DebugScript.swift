#if DEBUG
import AppKit
import SwiftUI

/// Debug builds only: applies UI steps from `COMMA_DEBUG_SCRIPT` to the first window, so the
/// sort, filter and find UI can be checked from the command line. Steps are separated by `;`:
/// `sort=<column>:a|d`, `filter=<column|*>:<op>:<value>`, `find=<text>`, `replace`, `exportexcel`,
/// `select=<row>,<column>:<row>,<column>`, `paste=<text>`, `filldown`, `action=<selector>`,
/// `exportas=<0 JSON…4 XLSX>`, `fixall=<0 unclosed quote…3 extra fields>`, `save`,
/// `cardedit=<column>:<value>` (types into the record card and leaves the field),
/// `lines=<n>` (text wrapping), `endedit`, `dump` (logs the geometry of the tables),
/// `formula=<text>` and `formulaapply` (the computed column sheet).
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
            case "select" where parts.count == 2:
                // select=<row>,<column>:<row>,<column> (anchor:cursor)
                let cell = { (text: String) -> GridCell in
                    let numbers = text.split(separator: ",").compactMap { Int($0) }
                    return GridCell(row: numbers.first ?? 0, column: numbers.last ?? 0)
                }
                grid.select(GridSelection(anchor: cell(parts[0]), cursor: cell(parts[1])))
            case "paste":
                // \t and \n in the argument stand for a tab and a line break.
                grid.paste(text: argument.replacingOccurrences(of: "\\t", with: "\t").replacingOccurrences(of: "\\n", with: "\n"))
            case "filldown":
                grid.fillDown(nil)
            case "exportas":
                let item = NSMenuItem()
                item.tag = Int(argument) ?? 0
                grid.exportAs(item)
            case "action":
                // action=<selector>, e.g. action=trimSpaces: — any command of the grid.
                NSApp.sendAction(Selector(argument), to: grid, from: nil)
            case "fixall":
                let kinds: [ProblemKind] = [.unclosedQuote, .strayQuote, .missingFields, .extraFields]
                grid.document.fixAll(kinds[min(Int(argument) ?? 0, kinds.count - 1)])
            case "cardedit" where parts.count == 2:
                if let field = grid.cardModel.fields.first(where: { $0.column == Int(parts[0]) }) {
                    grid.cardModel.draftChanged(field, to: parts[1])
                }
            case "lines":
                let item = NSMenuItem()
                item.tag = Int(argument) ?? 1
                grid.setLinesPerRow(item)
            case "endedit":
                // Leaves the cell being edited, as clicking elsewhere does.
                grid.view.window?.makeFirstResponder(grid.tableView)
            case "formula", "formulaapply":
                // Types into the computed column sheet, or applies it.
                let sheet = grid.presentedViewControllers?.compactMap { $0 as? NSHostingController<FormulaView> }.first
                if name == "formula" {
                    sheet?.rootView.model.formula = argument
                } else {
                    sheet?.rootView.model.onApply?()
                }
            case "dump":
                // Geometry of both tables, for layout problems.
                for table in grid.tables {
                    let widths = table.tableColumns.map { "\($0.identifier.rawValue)=\(Int($0.width))" }
                    NSLog("COMMA table frame=\(table.frame) clip=\(table.enclosingScrollView?.frame ?? .zero) \(widths)")
                }
            case "save":
                grid.document.save(nil)
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
