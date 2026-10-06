import AppKit

// Data menu: text transforms, filling empty cells, splitting and merging columns, duplicates.

extension GridViewController {
    /// The selection, or the whole column of the active cell when one cell is selected.
    private var operationRange: CellRange? {
        guard let selection, dataRowCount > 0 else { return nil }
        guard selection.isSingleCell else { return selection.range }
        let column = UInt64(selection.cursor.column)
        return CellRange(top: 0, left: column, bottom: UInt64(dataRowCount - 1), right: column)
    }

    private func transform(_ transform: TextTransform, _ sender: Any?) {
        guard let range = operationRange else { return }
        document.transform(range, transform, actionName: L("Text Change"))
    }

    @objc func trimSpaces(_ sender: Any?) { transform(.trim, sender) }
    @objc func collapseSpaces(_ sender: Any?) { transform(.collapseSpaces, sender) }
    @objc func makeUppercase(_ sender: Any?) { transform(.uppercase, sender) }
    @objc func makeLowercase(_ sender: Any?) { transform(.lowercase, sender) }
    @objc func capitalizeWords(_ sender: Any?) { transform(.capitalize, sender) }

    @objc func fillEmptyFromAbove(_ sender: Any?) {
        guard let range = operationRange else { return }
        document.fillEmpty(range, fromAbove: true, value: "")
    }

    @objc func fillEmptyWithValue(_ sender: Any?) {
        guard let range = operationRange else { return }
        ask(L("Fill Empty Cells"), L("Value for the empty cells of the selection:"), text: "") { [weak self] value, _ in
            self?.document.fillEmpty(range, fromAbove: false, value: value)
        }
    }

    @objc func splitColumn(_ sender: Any?) {
        let column = activeColumn
        ask(
            L("Split Column “%@”", columnTitle(column)),
            L("The parts go to new columns to the right; the column itself stays."),
            text: "",
            placeholder: L("empty: split at spaces"),
            label: L("Separator:"),
            checkbox: L("Regular expression")
        ) { [weak self] separator, regex in
            guard let self else { return }
            // An empty separator splits at runs of whitespace.
            let (pattern, isRegex) = separator.isEmpty ? (#"\s+"#, true) : (separator, regex)
            if self.document.splitColumn(column, separator: pattern, regex: isRegex) == 0 {
                self.inform(L("Nothing to split"), L("No value in this column contains the separator."))
            }
        }
    }

    @objc func mergeColumns(_ sender: Any?) {
        guard let selection, selection.columns.count > 1 else { return }
        let columns = Array(selection.columns)
        let names = columns.map(columnTitle).joined(separator: ", ")
        ask(
            L("Merge Columns"),
            L("Joins %@ into a new column after them; empty values are skipped.", names),
            text: "",
            placeholder: L("empty: a space"),
            label: L("Separator:")
        ) { [weak self] separator, _ in
            self?.document.mergeColumns(columns, separator: separator.isEmpty ? " " : separator)
        }
    }

    @objc func removeDuplicateRows(_ sender: Any?) {
        let selectedColumns = selection.map { Array($0.columns) } ?? []
        let byColumns = selectedColumns.count < columnCount && !selectedColumns.isEmpty
        let scope = byColumns
            ? L("Rows count as duplicates when they match in %@.", selectedColumns.map(columnTitle).joined(separator: ", "))
            : L("Rows count as duplicates when they match in every column.")
        ask(
            L("Remove Duplicate Rows"),
            scope + " " + L("The first of each set of equal rows stays."),
            checkbox: L("Ignore case and surrounding spaces")
        ) { [weak self] _, normalize in
            guard let self else { return }
            let removed = self.document.removeDuplicates(columns: byColumns ? selectedColumns : [], normalize: normalize)
            self.inform(
                removed == 0 ? L("No duplicates found") : L("Rows removed: %@", Self.format(removed)),
                removed == 0 ? "" : L("Choose Edit ▸ Undo to bring them back.")
            )
        }
    }

    // MARK: Dialogs

    /// A sheet with an optional text field and checkbox; `completion` gets their values on OK.
    private func ask(
        _ title: String,
        _ message: String,
        text: String? = nil,
        placeholder: String? = nil,
        label: String? = nil,
        checkbox: String? = nil,
        completion: @escaping (String, Bool) -> Void
    ) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: L("OK"))
        alert.addButton(withTitle: L("Cancel"))

        let field = NSTextField(string: text ?? "")
        field.placeholderString = placeholder
        let check = NSButton(checkboxWithTitle: checkbox ?? "", target: nil, action: nil)
        var rows: [NSView] = []
        if text != nil {
            field.widthAnchor.constraint(equalToConstant: 240).isActive = true
            rows.append(label.map { NSStackView(views: [NSTextField(labelWithString: $0), field]) } ?? field)
        }
        if checkbox != nil {
            rows.append(check)
        }
        if !rows.isEmpty {
            let stack = NSStackView(views: rows)
            stack.orientation = .vertical
            stack.alignment = .leading
            stack.spacing = 8
            stack.frame.size = stack.fittingSize
            alert.accessoryView = stack
            if text != nil {
                alert.window.initialFirstResponder = field
            }
        }
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn else { return }
            completion(field.stringValue, check.state == .on)
        }
    }

    private func inform(_ title: String, _ message: String) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.beginSheetModal(for: window)
    }
}
