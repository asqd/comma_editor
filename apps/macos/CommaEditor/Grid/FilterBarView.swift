import AppKit

@MainActor
protocol FilterBarDelegate: AnyObject {
    func filterBarDidChange(_ bar: FilterBarView)
    func filterBarClose(_ bar: FilterBarView)
}

/// Filter conditions above the table, combined with AND.
final class FilterBarView: NSView {
    weak var delegate: FilterBarDelegate?

    private let rowsStack = NSStackView()
    private let summaryLabel = NSTextField(labelWithString: "")
    private var columnTitles: [String] = []
    private var rows: [FilterRowView] { rowsStack.arrangedSubviews.compactMap { $0 as? FilterRowView } }
    private var debounce: Timer?

    /// Complete conditions; rows still waiting for a value are left out.
    var rules: [FilterRule] {
        rows.compactMap(\.rule)
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        rowsStack.orientation = .vertical
        rowsStack.alignment = .leading
        rowsStack.spacing = 4

        summaryLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        summaryLabel.textColor = .secondaryLabelColor
        let done = NSButton(title: L("Done"), target: self, action: #selector(close(_:)))
        let trailing = NSStackView(views: [summaryLabel, done])
        trailing.spacing = 8

        let content = NSStackView(views: [rowsStack, NSView(), trailing])
        content.orientation = .horizontal
        content.alignment = .top
        content.edgeInsets = NSEdgeInsets(top: 6, left: 10, bottom: 6, right: 10)
        // The bar is exactly as tall as its rows; the table takes the rest of the window.
        content.setHuggingPriority(.required, for: .vertical)
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: topAnchor),
            content.bottomAnchor.constraint(equalTo: bottomAnchor),
            content.leadingAnchor.constraint(equalTo: leadingAnchor),
            content.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
        addRow(after: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func setColumns(_ titles: [String]) {
        columnTitles = titles
        rows.forEach { $0.setColumns(titles) }
    }

    /// Shows `rules`, replacing what the rows hold. An empty list leaves one blank row.
    func setRules(_ rules: [FilterRule]) {
        rows.forEach { $0.removeFromSuperview() }
        for rule in rules {
            addRow(after: nil).rule = rule
        }
        if rules.isEmpty {
            addRow(after: nil)
        }
    }

    /// Starts a condition on `column`: fills the first empty row or adds one, and focuses it.
    func beginRule(column: Int) {
        let row = rows.first { $0.isBlank } ?? addRow(after: rows.last)
        row.selectColumn(column)
        row.focus()
    }

    func setSummary(_ text: String) {
        summaryLabel.stringValue = text
    }

    func focusFirstRow() {
        rows.first?.focus()
    }

    @discardableResult
    private func addRow(after row: FilterRowView?) -> FilterRowView {
        let newRow = FilterRowView(columns: columnTitles)
        newRow.onChange = { [weak self] immediate in self?.rowsChanged(immediate: immediate) }
        newRow.onAdd = { [weak self, weak newRow] in
            guard let self, let newRow else { return }
            self.addRow(after: newRow).focus()
        }
        newRow.onRemove = { [weak self, weak newRow] in
            guard let self, let newRow else { return }
            self.remove(newRow)
        }
        if let row, let index = rowsStack.arrangedSubviews.firstIndex(of: row) {
            rowsStack.insertArrangedSubview(newRow, at: index + 1)
        } else {
            rowsStack.addArrangedSubview(newRow)
        }
        return newRow
    }

    private func remove(_ row: FilterRowView) {
        if rows.count == 1 {
            row.clear()
        } else {
            row.removeFromSuperview()
        }
        rowsChanged(immediate: true)
    }

    private func rowsChanged(immediate: Bool) {
        debounce?.invalidate()
        guard !immediate else {
            delegate?.filterBarDidChange(self)
            return
        }
        debounce = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.delegate?.filterBarDidChange(self)
            }
        }
    }

    @objc private func close(_ sender: Any?) {
        delegate?.filterBarClose(self)
    }
}

/// One condition: column, operator, value, and buttons to remove it or add another.
private final class FilterRowView: NSStackView, NSTextFieldDelegate {
    /// The argument is `true` for changes to apply at once rather than after typing pauses.
    var onChange: ((Bool) -> Void)?
    var onAdd: (() -> Void)?
    var onRemove: (() -> Void)?

    private let columnPopUp = NSPopUpButton()
    private let opPopUp = NSPopUpButton()
    private let valueField = NSTextField()

    private static let operators: [(FilterOp, String)] = [
        (.contains, L("contains")),
        (.notContains, L("does not contain")),
        (.equals, L("is")),
        (.notEquals, L("is not")),
        (.startsWith, L("begins with")),
        (.endsWith, L("ends with")),
        (.matches, L("matches regex")),
        (.greaterThan, L("is greater than")),
        (.lessThan, L("is less than")),
        (.isEmpty, L("is empty")),
        (.isNotEmpty, L("is not empty")),
    ]

    var rule: FilterRule? {
        get {
            let op = Self.operators[opPopUp.indexOfSelectedItem].0
            let needsValue = op != .isEmpty && op != .isNotEmpty
            if needsValue && valueField.stringValue.isEmpty {
                return nil
            }
            let column = columnPopUp.indexOfSelectedItem
            return FilterRule(
                column: column >= 2 ? UInt64(column - 2) : nil,
                op: op,
                value: needsValue ? valueField.stringValue : "",
                caseSensitive: false
            )
        }
        set {
            guard let newValue else { return clear() }
            columnPopUp.selectItem(at: newValue.column.map { Int($0) + 2 } ?? 0)
            opPopUp.selectItem(at: Self.operators.firstIndex { $0.0 == newValue.op } ?? 0)
            valueField.stringValue = newValue.value
            updateValueField()
        }
    }

    init(columns: [String]) {
        super.init(frame: .zero)
        orientation = .horizontal
        spacing = 6
        alignment = .centerY
        columnPopUp.setAccessibilityLabel(L("Column"))
        opPopUp.setAccessibilityLabel(L("Condition"))
        valueField.setAccessibilityLabel(L("Value"))

        for popUp in [columnPopUp, opPopUp] {
            popUp.target = self
            popUp.action = #selector(popUpChanged(_:))
        }
        for (_, title) in Self.operators {
            opPopUp.addItem(withTitle: title)
        }
        valueField.placeholderString = L("Value")
        valueField.delegate = self
        valueField.widthAnchor.constraint(equalToConstant: 200).isActive = true

        let remove = Self.symbolButton("minus", L("Remove Condition"), #selector(removeTapped(_:)), self)
        let add = Self.symbolButton("plus", L("Add Condition"), #selector(addTapped(_:)), self)
        setViews([columnPopUp, opPopUp, valueField, remove, add], in: .leading)
        setColumns(columns)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func setColumns(_ titles: [String]) {
        let selected = columnPopUp.indexOfSelectedItem
        columnPopUp.removeAllItems()
        columnPopUp.addItem(withTitle: L("Any Column"))
        columnPopUp.menu?.addItem(.separator())
        for title in titles {
            columnPopUp.addItem(withTitle: title)
        }
        columnPopUp.selectItem(at: selected > 1 && selected < columnPopUp.numberOfItems ? selected : 0)
    }

    var isBlank: Bool {
        valueField.stringValue.isEmpty && Self.operators[opPopUp.indexOfSelectedItem].0 != .isEmpty
            && Self.operators[opPopUp.indexOfSelectedItem].0 != .isNotEmpty
    }

    func selectColumn(_ column: Int) {
        columnPopUp.selectItem(at: column + 2)
    }

    func clear() {
        columnPopUp.selectItem(at: 0)
        opPopUp.selectItem(at: 0)
        valueField.stringValue = ""
        updateValueField()
    }

    func focus() {
        window?.makeFirstResponder(valueField)
    }

    private func updateValueField() {
        let op = Self.operators[opPopUp.indexOfSelectedItem].0
        valueField.isEnabled = op != .isEmpty && op != .isNotEmpty
    }

    @objc private func popUpChanged(_ sender: Any?) {
        updateValueField()
        onChange?(true)
    }

    @objc private func removeTapped(_ sender: Any?) {
        onRemove?()
    }

    @objc private func addTapped(_ sender: Any?) {
        onAdd?()
    }

    func controlTextDidChange(_ notification: Notification) {
        onChange?(false)
    }

    private static func symbolButton(_ symbol: String, _ label: String, _ action: Selector, _ target: AnyObject) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)!
        let button = NSButton(image: image, target: target, action: action)
        button.bezelStyle = .circular
        button.controlSize = .small
        button.toolTip = label
        return button
    }
}
