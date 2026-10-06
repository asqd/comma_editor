import AppKit

/// Sheet for editing a value that spans several lines.
final class CellEditorViewController: NSViewController {
    private let heading: String
    private let initialValue: String
    private let onSave: (String) -> Void
    private let textView = NSTextView()

    init(heading: String, value: String, onSave: @escaping (String) -> Void) {
        self.heading = heading
        self.initialValue = value
        self.onSave = onSave
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 560, height: 340))

        let title = NSTextField(labelWithString: heading)
        title.font = .boldSystemFont(ofSize: NSFont.systemFontSize)

        textView.isRichText = false
        textView.allowsUndo = true
        textView.font = .systemFont(ofSize: NSFont.systemFontSize)
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.string = initialValue

        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder
        scrollView.documentView = textView

        let hint = NSTextField(labelWithString: L("Line breaks are kept. Press ⌘↩ to save."))
        hint.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        hint.textColor = .secondaryLabelColor

        let cancel = NSButton(title: L("Cancel"), target: self, action: #selector(cancel(_:)))
        cancel.keyEquivalent = "\u{1b}"
        let save = NSButton(title: L("Save"), target: self, action: #selector(save(_:)))
        save.keyEquivalent = "\r"
        save.keyEquivalentModifierMask = .command

        for subview in [title, scrollView, hint, cancel, save] as [NSView] {
            subview.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(subview)
        }
        NSLayoutConstraint.activate([
            title.topAnchor.constraint(equalTo: root.topAnchor, constant: 16),
            title.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
            title.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -20),
            scrollView.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 10),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -20),
            scrollView.bottomAnchor.constraint(equalTo: save.topAnchor, constant: -14),
            hint.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
            hint.centerYAnchor.constraint(equalTo: save.centerYAnchor),
            save.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -20),
            save.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -16),
            cancel.trailingAnchor.constraint(equalTo: save.leadingAnchor, constant: -8),
            cancel.centerYAnchor.constraint(equalTo: save.centerYAnchor),
        ])
        view = root
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(textView)
    }

    @objc private func save(_ sender: Any?) {
        let value = textView.string
        dismiss(nil)
        onSave(value)
    }

    @objc private func cancel(_ sender: Any?) {
        dismiss(nil)
    }
}
