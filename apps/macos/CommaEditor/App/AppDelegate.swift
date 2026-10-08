import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = MainMenu.make()
    }

    /// Instead of an empty untitled table, launching the app (or clicking its Dock icon with no
    /// windows open) shows the Open panel: people come here to edit an existing file.
    #if DEBUG
    /// Logs errors as they are shown, so that debug scripts can check their wording.
    func application(_ application: NSApplication, willPresentError error: Error) -> Error {
        let error = error as NSError
        NSLog("COMMA presenting \(error.localizedDescription) | \(error.localizedRecoverySuggestion ?? "-")")
        return error
    }
    #endif

    func applicationOpenUntitledFile(_ sender: NSApplication) -> Bool {
        NSDocumentController.shared.openDocument(nil)
        return true
    }
}
