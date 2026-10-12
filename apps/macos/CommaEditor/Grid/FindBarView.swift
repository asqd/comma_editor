import AppKit

@MainActor
protocol FindBarDelegate: AnyObject {
    /// The query or its options changed while typing.
    func findBarQueryDidChange(_ bar: FindBarView)
    func findBar(_ bar: FindBarView, findForward forward: Bool)
    func findBarReplace(_ bar: FindBarView)
    func findBarReplaceAll(_ bar: FindBarView)
    func findBarClose(_ bar: FindBarView)
}

/// Find bar above the table, with an optional replace row.
final class FindBarView: NSView, NSSearchFieldDelegate {
    weak var delegate: FindBarDelegate?

    let searchField = NSSearchField()
    let replaceField = NSTextField()
    private let caseSensitive = NSButton(checkboxWithTitle: L("Match Case"), target: nil, action: nil)
    private let wholeCell = NSButton(checkboxWithTitle: L("Whole Cell"), target: nil, action: nil)
    private let regex = NSButton(checkboxWithTitle: L("Regex"), target: nil, action: nil)
    private let columnPopUp = NSPopUpButton()
    private let resultLabel = NSTextField(labelWithString: "")
    private let replaceRow = NSStackView()
    private var debounce: Timer?
    /// The cells "In Selection" searches: the selection when that scope was chosen.
    private(set) var selectionScope: CellRange?
    /// Supplies the current selection when "In Selection" is chosen.
    var currentSelection: (() -> CellRange?)?

    /// Menu layout: All Columns, In Selection, a separator, then the columns.
    private static let selectionItem = 1
    private static let firstColumnItem = 3

    var isReplaceVisible: Bool {
        get { !replaceRow.isHidden }
        set { replaceRow.isHidden = !newValue }
    }

    var query: SearchQuery {
        let item = columnPopUp.indexOfSelectedItem
        return SearchQuery(
            text: searchField.stringValue,
            caseSensitive: caseSensitive.state == .on,
            wholeCell: wholeCell.state == .on,
            regex: regex.state == .on,
            column: item >= Self.firstColumnItem ? UInt64(item - Self.firstColumnItem) : nil,
            range: item == Self.selectionItem ? selectionScope : nil
        )
    }

    /// Searches only `range`, or everywhere again with `nil`.
    func setSelectionScope(_ range: CellRange?) {
        selectionScope = range
        updateSelectionItem()
        columnPopUp.selectItem(at: range == nil ? 0 : Self.selectionItem)
    }

    private func updateSelectionItem() {
        guard let item = columnPopUp.item(at: Self.selectionItem) else { return }
        if let range = selectionScope {
            let rows = GridViewController.format(Int(range.bottom - range.top) + 1)
            let columns = GridViewController.format(Int(range.right - range.left) + 1)
            item.title = L("In Selection (%@ × %@)", rows, columns)
        } else {
            item.title = L("In Selection")
        }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)

        searchField.placeholderString = L("Find")
        columnPopUp.setAccessibilityLabel(L("Search in"))
        searchField.sendsSearchStringImmediately = true
        searchField.delegate = self
        // The clear button sends the action without a text change notification.
        searchField.target = self
        searchField.action = #selector(searchFieldAction(_:))
        replaceField.placeholderString = L("Replace with")
        replaceField.delegate = self
        for checkbox in [caseSensitive, wholeCell, regex] {
            checkbox.target = self
            checkbox.action = #selector(optionsChanged(_:))
            checkbox.controlSize = .small
            checkbox.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        }
        regex.toolTip = L("Use a regular expression; replacements can refer to groups as $1.")
        columnPopUp.target = self
        columnPopUp.action = #selector(optionsChanged(_:))
        columnPopUp.controlSize = .small
        columnPopUp.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        resultLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        resultLabel.textColor = .secondaryLabelColor
        resultLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let navigation = NSSegmentedControl(
            images: [
                NSImage(systemSymbolName: "chevron.left", accessibilityDescription: L("Find Previous"))!,
                NSImage(systemSymbolName: "chevron.right", accessibilityDescription: L("Find Next"))!,
            ],
            trackingMode: .momentary,
            target: self,
            action: #selector(navigate(_:))
        )
        navigation.segmentStyle = .separated
        let done = NSButton(title: L("Done"), target: self, action: #selector(close(_:)))
        done.bezelStyle = .rounded
        let replace = NSButton(title: L("Replace"), target: self, action: #selector(replace(_:)))
        let replaceAll = NSButton(title: L("Replace All"), target: self, action: #selector(replaceAll(_:)))

        let findRow = NSStackView(views: [
            searchField, navigation, caseSensitive, wholeCell, regex, columnPopUp, resultLabel, NSView(), done,
        ])
        replaceRow.setViews([replaceField, replace, replaceAll], in: .leading)
        for row in [findRow, replaceRow] {
            row.orientation = .horizontal
            row.spacing = 8
            row.alignment = .centerY
        }
        let rows = NSStackView(views: [findRow, replaceRow])
        rows.orientation = .vertical
        rows.alignment = .leading
        rows.spacing = 6
        rows.edgeInsets = NSEdgeInsets(top: 6, left: 10, bottom: 6, right: 10)
        // The bar is exactly as tall as its rows; the table takes the rest of the window.
        rows.setHuggingPriority(.required, for: .vertical)
        rows.translatesAutoresizingMaskIntoConstraints = false
        addSubview(rows)
        NSLayoutConstraint.activate([
            rows.topAnchor.constraint(equalTo: topAnchor),
            rows.bottomAnchor.constraint(equalTo: bottomAnchor),
            rows.leadingAnchor.constraint(equalTo: leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: trailingAnchor),
            findRow.trailingAnchor.constraint(equalTo: rows.trailingAnchor, constant: -10),
            searchField.widthAnchor.constraint(equalToConstant: 260),
            replaceField.widthAnchor.constraint(equalTo: searchField.widthAnchor),
        ])
        replaceRow.isHidden = true
        setColumns([])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    /// Fills the column scope menu, keeping the selection when the column still exists.
    func setColumns(_ titles: [String]) {
        let selected = columnPopUp.indexOfSelectedItem
        columnPopUp.removeAllItems()
        columnPopUp.addItem(withTitle: L("All Columns"))
        columnPopUp.addItem(withTitle: L("In Selection"))
        columnPopUp.menu?.addItem(.separator())
        for title in titles {
            columnPopUp.addItem(withTitle: title)
        }
        updateSelectionItem()
        // `query` maps the items back to a scope.
        let keep = selected == Self.selectionItem || (selected >= Self.firstColumnItem && selected < columnPopUp.numberOfItems)
        columnPopUp.selectItem(at: keep ? selected : 0)
    }

    #if DEBUG
    /// The scope and the result shown, for debug scripts.
    var debugState: String { "\(columnPopUp.titleOfSelectedItem ?? "-") · \(resultLabel.stringValue)" }
    #endif

    func showResult(_ text: String, isError: Bool = false) {
        resultLabel.stringValue = text
        resultLabel.textColor = isError ? .systemRed : .secondaryLabelColor
    }

    // MARK: Actions

    @objc private func searchFieldAction(_ sender: NSSearchField) {
        if sender.stringValue.isEmpty {
            delegate?.findBarQueryDidChange(self)
        }
    }

    @objc private func optionsChanged(_ sender: Any?) {
        if sender as? NSPopUpButton === columnPopUp, columnPopUp.indexOfSelectedItem == Self.selectionItem {
            // The cells selected now; finding moves the cursor and changes the selection.
            guard let range = currentSelection?() else {
                NSSound.beep()
                setSelectionScope(nil)
                delegate?.findBarQueryDidChange(self)
                return
            }
            setSelectionScope(range)
        }
        delegate?.findBarQueryDidChange(self)
    }

    @objc private func navigate(_ sender: NSSegmentedControl) {
        delegate?.findBar(self, findForward: sender.selectedSegment == 1)
    }

    @objc private func replace(_ sender: Any?) {
        delegate?.findBarReplace(self)
    }

    @objc private func replaceAll(_ sender: Any?) {
        delegate?.findBarReplaceAll(self)
    }

    @objc private func close(_ sender: Any?) {
        delegate?.findBarClose(self)
    }

    func controlTextDidChange(_ notification: Notification) {
        guard notification.object as? NSSearchField === searchField else { return }
        debounce?.invalidate()
        debounce = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.delegate?.findBarQueryDidChange(self)
            }
        }
    }

    /// Return finds the next match (Shift-Return the previous one); Escape closes the bar.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.insertNewline(_:)):
            if control === replaceField {
                delegate?.findBarReplace(self)
            } else {
                let backwards = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
                delegate?.findBar(self, findForward: !backwards)
            }
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            delegate?.findBarClose(self)
            return true
        default:
            return false
        }
    }
}
