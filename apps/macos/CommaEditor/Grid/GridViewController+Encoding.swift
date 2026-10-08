import AppKit
import SwiftUI

// File ▸ Check Encoding and Delimiter: the sheet that shows how the file reads with another
// encoding or delimiter and reopens it so. It opens by itself when detection is not sure.

extension GridViewController {
    /// File ▸ Compare with File: another file against this table, in a window of its own.
    @objc func compareWithFile(_ sender: Any?) {
        guard !document.isIndexing else {
            NSSound.beep()
            return
        }
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.message = L("Choose a file to compare with “%@”.", document.displayName ?? "")
        panel.prompt = L("Compare")
        panel.directoryURL = document.fileURL?.deletingLastPathComponent()
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url, let self else { return }
            DiffWindowController.compare(self, with: url)
        }
    }

    @objc func showOpenOptions(_ sender: Any?) {
        presentOpenOptions(uncertain: false)
    }

    /// Once per window, right after a file opens whose encoding or delimiter is a guess.
    func checkDetection() {
        guard !info.detectionConfident, document.fileURL != nil, !askedAboutDetection else { return }
        askedAboutDetection = true
        presentOpenOptions(uncertain: true)
    }

    private func presentOpenOptions(uncertain: Bool) {
        guard let url = document.fileURL else { return }
        let model = OpenOptionsModel(
            path: url.path,
            encoding: document.chosenEncoding,
            delimiter: document.chosenDelimiter,
            uncertain: uncertain
        )
        let controller = NSHostingController(rootView: OpenOptionsView(model: model))
        model.onCancel = { [weak self, weak controller] in
            guard let controller else { return }
            self?.dismiss(controller)
        }
        model.onApply = { [weak self, weak controller] encoding, delimiter in
            guard let self, let controller else { return }
            self.dismiss(controller)
            self.document.confirmReopen { $0.reopen(encoding: encoding, delimiter: delimiter) }
        }
        presentAsSheet(controller)
    }
}
