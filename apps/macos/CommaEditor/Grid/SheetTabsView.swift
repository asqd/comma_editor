import AppKit

/// Tabs of the sheets of a workbook, in the order Excel shows them. Hidden sheets are listed
/// dimmed; sheets that are charts, not tables, cannot be chosen.
final class SheetTabsView: NSView {
    var onSelect: ((Int) -> Void)?

    private let scrollView = NSScrollView()
    private let stack = NSStackView()
    private var buttons: [NSButton] = []
    private var shownSheets: [SheetInfo] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        stack.orientation = .horizontal
        stack.spacing = 2
        stack.edgeInsets = NSEdgeInsets(top: 3, left: 8, bottom: 3, right: 8)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let documentView = FlippedView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(stack)
        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.setAccessibilityRole(.radioGroup)
        scrollView.setAccessibilityLabel(L("Sheets"))
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.horizontalScrollElasticity = .allowed
        scrollView.verticalScrollElasticity = .none
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: documentView.topAnchor),
            stack.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            documentView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            documentView.bottomAnchor.constraint(equalTo: scrollView.contentView.bottomAnchor),
            documentView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            heightAnchor.constraint(equalToConstant: 28),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    /// Shows the sheets with `selected` chosen; `loading` is a sheet being read.
    func update(sheets: [SheetInfo], selected: Int, loading: Int?) {
        if sheets != shownSheets {
            shownSheets = sheets
            for button in buttons {
                stack.removeArrangedSubview(button)
                button.removeFromSuperview()
            }
            buttons = sheets.enumerated().map { makeButton(for: $0.element, index: $0.offset) }
            for button in buttons {
                stack.addArrangedSubview(button)
            }
        }
        for (index, button) in buttons.enumerated() {
            button.state = index == selected ? .on : .off
            button.attributedTitle = title(of: sheets[index], loading: index == loading)
        }
        if buttons.indices.contains(selected) {
            layoutSubtreeIfNeeded()
            buttons[selected].scrollToVisible(buttons[selected].bounds)
        }
    }

    private func makeButton(for sheet: SheetInfo, index: Int) -> NSButton {
        let button = NSButton(title: sheet.name, target: self, action: #selector(choose(_:)))
        button.setButtonType(.pushOnPushOff)
        button.bezelStyle = .accessoryBarAction
        button.controlSize = .small
        button.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        button.tag = index
        // VoiceOver reads the tabs as a choice of one sheet, not as checkboxes.
        button.setAccessibilityRole(.radioButton)
        button.isEnabled = sheet.isTable
        if !sheet.isTable {
            button.toolTip = L("A chart sheet: it has no cells to show")
        } else if sheet.visibility != .visible {
            button.toolTip = L("A hidden sheet")
        }
        return button
    }

    /// The name; dimmed and slanted for a hidden sheet, with an ellipsis while it is read.
    private func title(of sheet: SheetInfo, loading: Bool) -> NSAttributedString {
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        let hidden = sheet.isTable && sheet.visibility != .visible
        return NSAttributedString(string: loading ? sheet.name + " …" : sheet.name, attributes: [
            .font: hidden ? NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) : font,
            .foregroundColor: hidden || !sheet.isTable ? NSColor.secondaryLabelColor : NSColor.labelColor,
        ])
    }

    @objc private func choose(_ sender: NSButton) {
        // The button turns itself on and off; the shown sheet decides its state.
        sender.state = sender.state == .on ? .off : .on
        onSelect?(sender.tag)
    }
}

private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}
