import AppKit

final class DocumentWindowController: NSWindowController, NSWindowDelegate {
    /// Field editor for grid cells: no smart quotes, dashes or autocorrection, which would
    /// silently change the data.
    private lazy var plainFieldEditor: NSTextView = {
        let editor = NSTextView()
        editor.isFieldEditor = true
        editor.isRichText = false
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticTextReplacementEnabled = false
        editor.isAutomaticSpellingCorrectionEnabled = false
        editor.isContinuousSpellCheckingEnabled = false
        editor.isAutomaticLinkDetectionEnabled = false
        editor.isAutomaticDataDetectionEnabled = false
        editor.isAutomaticTextCompletionEnabled = false
        return editor
    }()

    let documentToolbar = DocumentToolbar()

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: true
        )
        window.minSize = NSSize(width: 520, height: 320)
        window.toolbarStyle = .unified
        window.toolbar = documentToolbar.toolbar
        window.center()
        super.init(window: window)
        window.delegate = self
        shouldCascadeWindows = true
        windowFrameAutosaveName = "DocumentWindow"
        // A frame saved smaller than the minimum (after a layout glitch) would hide the table.
        if window.frame.width < window.minSize.width || window.frame.height < window.minSize.height {
            window.setContentSize(NSSize(width: 1100, height: 700))
            window.center()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override var document: AnyObject? {
        didSet {
            guard let document = document as? CSVDocument, contentViewController == nil else { return }
            let frame = window?.frame
            let grid = GridViewController(document: document)
            grid.documentToolbar = documentToolbar
            documentToolbar.grid = grid
            contentViewController = grid
            if let frame {
                window?.setFrame(frame, display: false)
            }
        }
    }

    func windowWillReturnUndoManager(_ window: NSWindow) -> UndoManager? {
        (document as? NSDocument)?.undoManager
    }

    func windowWillReturnFieldEditor(_ sender: NSWindow, to client: Any?) -> Any? {
        client is NSTextField ? plainFieldEditor : nil
    }
}
