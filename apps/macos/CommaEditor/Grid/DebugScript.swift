#if DEBUG
import AppKit
import UniformTypeIdentifiers
import SwiftUI

/// Debug builds only: applies UI steps from `COMMA_DEBUG_SCRIPT` to the first window, so the
/// sort, filter and find UI can be checked from the command line. Steps are separated by `;`:
/// `sort=<column>:a|d`, `filter=<column|*>:<op>:<value>`, `find=<text>`, `replace`, `exportexcel`,
/// `select=<row>,<column>:<row>,<column>`, `paste=<text>`, `filldown`, `action=<selector>`,
/// `exportas=<0 JSON…4 XLSX>`, `fixall=<0 unclosed quote…3 extra fields>`, `save`,
/// `replaceall=<text>`, `cardedit=<column>:<value>` (types into the record card and leaves the field),
/// `lines=<n>` (text wrapping), `endedit`, `dump` (logs the geometry of the tables),
/// `formula=<text>` and `formulaapply` (the computed column sheet), `cancel` (stops background
/// work), `undo`, `compare=<left>:<right>` and `diff=<step>` (comparison window),
/// `sheet=<index>` (a sheet of a workbook), `exportto=<path>` (Export as CSV), `wait=<seconds>`,
/// `state=<rows>` (logs the table: titles, rows, selection, status bar), `menu=<selector>` (logs
/// whether the menu item is enabled), `alert` (logs the texts of the sheet or panel shown),
/// `press=<button title>` (clicks it), `redo`, `window=<file name>` (runs the next steps on the
/// window of another open file).
/// Example: `open --env COMMA_DEBUG_SCRIPT='sort=2:d;find=lon' -a CommaEditor file.csv`
@MainActor
enum DebugScript {
    private static var done = false
    /// `grouping=off` leaves undo groups to the undo manager, as for long edits that finish in
    /// the background and register their undo later.
    private static var groupsSteps = true

    static func run(on grid: GridViewController) {
        guard !done, let script = ProcessInfo.processInfo.environment["COMMA_DEBUG_SCRIPT"] else { return }
        done = true
        // `\;` stands for a semicolon inside a step, as in `paste==IF(A1>0\;1\;2)`.
        let steps = script.replacingOccurrences(of: "\\;", with: "\u{1}").split(separator: ";")
            .map { $0.replacingOccurrences(of: "\u{1}", with: ";") }
        run(steps[...], on: grid)
    }

    /// Runs steps in order; `wait=<seconds>` runs the rest later.
    private static func run(_ steps: ArraySlice<String>, on grid: GridViewController) {
        let ops: [String: FilterOp] = [
            "contains": .contains, "equals": .equals, "notequals": .notEquals,
            "greater": .greaterThan, "less": .lessThan, "empty": .isEmpty, "regex": .matches,
        ]
        for (index, step) in zip(steps.indices, steps) {
            let (name, argument) = step.split(separator: "=", maxSplits: 1).map(String.init).pair
            if name == "wait" {
                let rest = steps[(index + 1)...]
                DispatchQueue.main.asyncAfter(deadline: .now() + (Double(argument) ?? 1)) { [weak grid] in
                    if let grid { run(rest, on: grid) }
                }
                return
            }
            let parts = argument.split(separator: ":", maxSplits: 2).map(String.init)
            // A step runs outside an event, where the undo manager opens no group of its own:
            // without one, edits would leave no undo step and the document would not look edited.
            let undoManager = grid.document.undoManager
            // Only edits: a new group, even an empty one, clears what Redo would bring back.
            let edits: Set<String> = ["paste", "filldown", "action", "fixall", "cardedit", "replaceall", "formulaapply", "recipeapply", "save", "exportto", "close", "docaction"]
            // A command that changes nothing would leave an empty group, which counts as a change.
            // Moving through the card commits a field being edited, which is an edit.
            let commitsCard = ["nextRecord:", "previousRecord:"].contains(argument)
            let navigates = name == "action" && !GridViewController.editingActions.contains(Selector(argument)) && !commitsCard
            let groups = groupsSteps && undoManager?.groupingLevel == 0 && edits.contains(name) && !navigates
            // The step's own group, with no group for the event around it: one for the event
            // would be closed by the event loop later, after the step closed it, and leave the
            // undo manager broken for an edit finishing in the background.
            if groups, let manager = undoManager {
                manager.groupsByEvent = false
                manager.beginUndoGrouping()
            }
            defer {
                if groups, let manager = undoManager {
                    while manager.groupingLevel > 0 {
                        manager.endUndoGrouping()
                    }
                    manager.groupsByEvent = true
                }
            }
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
            case "sheet":
                grid.workbook?.showSheet(Int(argument) ?? 0)
            case "exportto":
                // exportto=<path>: File ▸ Export as CSV without the panel.
                grid.document.save(
                    to: URL(fileURLWithPath: argument),
                    ofType: UTType.commaSeparatedText.identifier,
                    for: .saveToOperation
                ) { error in
                    NSLog("COMMA exported to \(argument): \(error.map { "\($0)" } ?? "ok")")
                }
            case "replaceall":
                // replaceall=<replacement>, for the text in the find field.
                grid.findBar.replaceField.stringValue = argument
                grid.findBarReplaceAll(grid.findBar)
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
                if grid.responds(to: Selector(argument)) {
                    NSApp.sendAction(Selector(argument), to: grid, from: nil)
                } else {
                    NSLog("%@", "COMMA action: the table has no \(argument)")
                }
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
            case "scrollbench":
                // scrollbench=<points a second>:<seconds>
                ScrollBench.run(
                    on: grid.tableView,
                    speed: CGFloat(Double(parts.first ?? "") ?? 3000),
                    duration: Double(parts.count > 1 ? parts[1] : "") ?? 3
                )
            case "exporttemp":
                grid.debugExport(GridViewController.ExportKind(rawValue: Int(argument) ?? 0) ?? .json)
            case "dump":
                // Geometry of both tables, for layout problems.
                for table in grid.tables {
                    let widths = table.tableColumns.map { "\($0.identifier.rawValue)=\(Int($0.width))" }
                    NSLog("COMMA table frame=\(table.frame) clip=\(table.enclosingScrollView?.frame ?? .zero) \(widths)")
                }
            case "compare" where parts.count == 2:
                // compare=<left file name>:<right file name>, both open in the app (the sandbox
                // lets it read files the user opened).
                let documents = NSDocumentController.shared.documents.compactMap { $0 as? CSVDocument }
                let named = { (name: String) in documents.first { $0.fileURL?.lastPathComponent == name } }
                if let left = named(parts[0]), let right = named(parts[1])?.fileURL,
                   let leftGrid = left.windowControllers.first?.contentViewController as? GridViewController {
                    DiffWindowController.compare(leftGrid, with: right)
                }
            case "diff":
                // diff=filter:<segment>, diff=key:<column or -1>, diff=case, diff=open
                DiffWindowController.debugStep(argument)
            case "recipestart":
                grid.document.startRecording()
            case "recipesave":
                // recipesave=<name>: stops recording and saves without the name dialog.
                if let recipe = grid.document.stopRecording() {
                    recipe.setName(name: argument)
                    _ = try? RecipeStore.shared.save(recipe)
                }
            case "recipeapply":
                // recipeapply=<recipe name>:<open file name>, or this table without a file name.
                let documents = NSDocumentController.shared.documents.compactMap { $0 as? CSVDocument }
                let target = parts.count > 1
                    ? documents.first { $0.fileURL?.lastPathComponent == parts[1] }
                    : grid.document
                if let recipe = RecipeStore.shared.all().first(where: { $0.name == parts[0] })?.recipe {
                    target?.applyRecipe(recipe)
                }
            case "recipes":
                grid.manageRecipes(nil)
            case "cancel":
                grid.cancelBackgroundWork(nil)
            case "undo":
                grid.undo(nil)
            case "save":
                grid.document.save(nil)
            case "close":
                grid.view.window?.performClose(nil)
            case "docaction":
                // docaction=<selector>: a command of the document, e.g. revertDocumentToSaved:.
                NSApp.sendAction(Selector(argument), to: grid.document, from: nil)
            case "row":
                // row=<visible row>: logs the values of one row, wherever it is.
                let row = Int(argument) ?? 0
                if row < grid.dataRowCount {
                    NSLog("%@", "COMMA row \(row): " + grid.values(forRow: row).joined(separator: " | "))
                }
            case "copyas":
                // copyas=<1 Markdown, 2 HTML…>: Edit ▸ Copy As.
                let item = NSMenuItem()
                item.tag = Int(argument) ?? 1
                grid.copyAs(item)
            case "reopen":
                // reopen=<index into CSVDocument.encodings, or -1 to detect>
                let item = NSMenuItem()
                item.tag = Int(argument) ?? -1
                grid.document.reopenWithEncoding(item)
            case "grouping":
                groupsSteps = argument != "off"
            case "activate":
                NSApp.activate(ignoringOtherApps: true)
                grid.view.window?.makeKeyAndOrderFront(nil)
            case "redo":
                grid.document.undoManager?.redo()
            case "state":
                logState(of: grid, rows: Int(argument) ?? 6)
            case "menu":
                logMenuItem(Selector(argument), grid: grid)
            case "alert":
                for window in [grid.view.window?.attachedSheet, NSApp.modalWindow].compactMap({ $0 }) {
                    NSLog("%@", "COMMA alert \(texts(in: window.contentView).joined(separator: " | "))")
                }
            case "press":
                let windows = [grid.view.window?.attachedSheet, NSApp.modalWindow].compactMap { $0 }
                if let button = windows.lazy.compactMap({ buttons(in: $0.contentView).first { $0.title == argument } }).first {
                    button.performClick(nil)
                } else {
                    NSLog("COMMA press: no button \(argument)")
                }
            case "window":
                let documents = NSDocumentController.shared.documents.compactMap { $0 as? CSVDocument }
                if let other = documents.first(where: { $0.fileURL?.lastPathComponent == argument }),
                   let otherGrid = other.windowControllers.first?.contentViewController as? GridViewController {
                    other.showWindows()
                    run(steps[(index + 1)...], on: otherGrid)
                    return
                }
                NSLog("COMMA window: no document \(argument)")
            case "split" where parts.count == 2:
                // split=<column>:<separator> — as the Split Column dialog does.
                grid.document.splitColumn(Int(parts[0]) ?? 0, separator: parts[1], regex: false) { count in
                    NSLog("%@", "COMMA split: \(count) columns")
                }
            case "merge" where parts.count == 2:
                // merge=<column>,<column>…:<separator> — as the Merge Columns dialog does.
                grid.document.mergeColumns(parts[0].split(separator: ",").compactMap { Int($0) }, separator: parts[1])
            case "appearance":
                // appearance=dark|light: the app's look, whatever the system's, for screenshots.
                NSApp.appearance = NSAppearance(named: argument == "dark" ? .darkAqua : .aqua)
            case "exportexcel":
                grid.document.exportForExcel(nil)
            default:
                NSLog("COMMA_DEBUG_SCRIPT: unknown step \(step)")
            }
        }
    }
}

extension DebugScript {
    /// The table as the window shows it, for checking a step without a screenshot.
    fileprivate static func logState(of grid: GridViewController, rows: Int) {
        let info = grid.core.info()
        var lines = ["window=\(grid.view.window?.title ?? "-") edited=\(grid.document.isDocumentEdited)"
            + " readOnly=\(grid.document.isReadOnly) header=\(info.hasHeader) rows=\(info.rowCount)"
            + " visible=\(info.visibleRowCount) columns=\(info.columnCount) active=\(NSApp.isActive)"
            + " key=\(grid.view.window?.isKeyWindow ?? false) changes=\(grid.document.hasUnautosavedChanges)"
            + " undo=\(grid.document.undoManager.map { "\($0.undoActionName)/\($0.redoActionName) level \($0.groupingLevel)" } ?? "-")"]
        if let workbook = grid.workbook {
            let name = workbook.sheets.indices.contains(workbook.sheetIndex) ? workbook.sheets[workbook.sheetIndex].name : "-"
            lines.append("sheet=\(name) csvMode=\(workbook.csvMode) mode=\(grid.sheetMode.selectedSegment)"
                + " modeEnabled=\(grid.sheetMode.isEnabled) sheets=\(workbook.sheets.map(\.name))")
        }
        let shown = grid.allTableColumns.filter { !$0.isHidden }
        lines.append("titles=" + shown.map(\.title).joined(separator: " | "))
        let hidden = (0..<grid.columnCount).filter { grid.hiddenColumns.contains($0) }
        lines.append("hiddenColumns=\(hidden) frozen=\(grid.frozenColumnCount)")
        for row in 0..<min(rows, grid.dataRowCount) {
            let fileRow = grid.core.visibleRecord(visibleRow: UInt64(row)).map { Int($0) } ?? -1
            let values = grid.values(forRow: row)
            let cells = values.indices.map { column in
                grid.formula(row: row, column: column).map { "\(values[column]) {=\($0)}" } ?? values[column]
            }
            lines.append("row \(grid.rowNumber(fileRow: fileRow)): " + cells.joined(separator: " | "))
        }
        lines.append("selection=\(grid.selection.map { "\($0.anchor.row),\($0.anchor.column):\($0.cursor.row),\($0.cursor.column)" } ?? "-")")
        lines.append("status=" + grid.statusLabel.stringValue)
        if !grid.findBar.isHidden, grid.findBar.window != nil {
            lines.append("find=" + grid.findBar.debugState)
        }
        if grid.busyLabel.superview?.isHidden == false {
            lines.append("busy=" + grid.busyLabel.stringValue)
        }
        for line in lines {
            NSLog("%@", "COMMA state \(line)")
        }
    }

    /// Whether the item for `action` is enabled, asked of the table and then of the document
    /// as the menu would ask them in a key window; launched from a script, the app may not get one.
    fileprivate static func logMenuItem(_ action: Selector, grid: GridViewController) {
        func find(_ menu: NSMenu) -> NSMenuItem? {
            menu.update()
            for item in menu.items {
                if item.action == action { return item }
                if let submenu = item.submenu, let found = find(submenu) { return found }
            }
            return nil
        }
        let item = NSApp.mainMenu.flatMap(find) ?? NSMenuItem(title: "?", action: action, keyEquivalent: "")
        let probe = NSMenuItem(title: item.title, action: action, keyEquivalent: "")
        probe.tag = item.tag
        let enabled: Bool
        if grid.responds(to: action) {
            enabled = grid.validateMenuItem(probe)
        } else if action == #selector(UndoManager.undo) || action == Selector(("undo:")) {
            enabled = grid.document.undoManager?.canUndo ?? false
        } else if action == Selector(("redo:")) {
            enabled = grid.document.undoManager?.canRedo ?? false
        } else if grid.document.responds(to: action) {
            enabled = grid.document.validateMenuItem(probe)
        } else {
            enabled = false
        }
        NSLog("%@", "COMMA menu \(action): “\(probe.title)” enabled=\(enabled) state=\(probe.state.rawValue)")
    }

    fileprivate static func texts(in view: NSView?) -> [String] {
        guard let view else { return [] }
        var found: [String] = []
        if let field = view as? NSTextField, !field.stringValue.isEmpty { found.append(field.stringValue) }
        if let button = view as? NSButton, !button.title.isEmpty { found.append("[\(button.title)]") }
        if let text = view as? NSTextView, !text.string.isEmpty { found.append(text.string) }
        return found + view.subviews.flatMap { texts(in: $0) }
    }

    fileprivate static func buttons(in view: NSView?) -> [NSButton] {
        guard let view else { return [] }
        return ((view as? NSButton).map { [$0] } ?? []) + view.subviews.flatMap { buttons(in: $0) }
    }
}

private extension Array where Element == String {
    var pair: (String, String) { (first ?? "", count > 1 ? self[1] : "") }
}
#endif
