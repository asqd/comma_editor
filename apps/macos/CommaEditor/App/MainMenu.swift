import AppKit

/// The main menu, built in code: the app has no storyboards or nibs.
@MainActor
enum MainMenu {
    static func make() -> NSMenu {
        let windowMenu = makeWindowMenu()
        let helpMenu = NSMenu(title: L("Help"))
        NSApp.windowsMenu = windowMenu
        NSApp.helpMenu = helpMenu

        let menu = NSMenu()
        for (title, submenu) in [
            (appName, makeAppMenu()),
            (L("File"), makeFileMenu()),
            (L("Edit"), makeEditMenu()),
            (L("Table"), makeTableMenu()),
            (L("View"), makeViewMenu()),
            (L("Window"), windowMenu),
            (L("Help"), helpMenu),
        ] {
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            submenu.title = title
            item.submenu = submenu
            menu.addItem(item)
        }
        return menu
    }

    private static var appName: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
            ?? ProcessInfo.processInfo.processName
    }

    private static func makeAppMenu() -> NSMenu {
        let services = NSMenu(title: L("Services"))
        NSApp.servicesMenu = services
        let servicesItem = NSMenuItem(title: L("Services"), action: nil, keyEquivalent: "")
        servicesItem.submenu = services

        return menu([
            item(L("About %@", appName), #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
            .separator(),
            servicesItem,
            .separator(),
            item(L("Hide %@", appName), #selector(NSApplication.hide(_:)), "h"),
            item(L("Hide Others"), #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            item(L("Show All"), #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item(L("Quit %@", appName), #selector(NSApplication.terminate(_:)), "q"),
        ])
    }

    private static func makeFileMenu() -> NSMenu {
        let recent = NSMenuItem(title: L("Open Recent"), action: nil, keyEquivalent: "")
        recent.submenu = RecentDocumentsMenu.shared.menu

        return menu([
            item(L("New"), #selector(NSDocumentController.newDocument(_:)), "n"),
            item(L("Open…"), #selector(NSDocumentController.openDocument(_:)), "o"),
            recent,
            .separator(),
            item(L("Close"), #selector(NSWindow.performClose(_:)), "w"),
            item(L("Save…"), #selector(NSDocument.save(_:)), "s"),
            item(L("Duplicate"), #selector(NSDocument.duplicate(_:)), "S"),
            item(L("Save As…"), #selector(NSDocument.saveAs(_:)), "S", [.command, .option]),
            item(L("Rename…"), #selector(NSDocument.rename(_:))),
            item(L("Move To…"), #selector(NSDocument.move(_:))),
            item(L("Export…"), #selector(NSDocument.saveTo(_:))),
            item(L("Export for Excel…"), #selector(CSVDocument.exportForExcel(_:)), "e", [.command, .shift]),
            item(L("Revert To Saved"), #selector(NSDocument.revertToSaved(_:))),
        ])
    }

    private static func makeEditMenu() -> NSMenu {
        menu([
            item(L("Undo"), Selector(("undo:")), "z"),
            item(L("Redo"), Selector(("redo:")), "Z"),
            .separator(),
            item(L("Cut"), #selector(NSText.cut(_:)), "x"),
            item(L("Copy"), #selector(NSText.copy(_:)), "c"),
            item(L("Paste"), #selector(NSText.paste(_:)), "v"),
            item(L("Delete"), #selector(NSText.delete(_:))),
            item(L("Select All"), #selector(NSText.selectAll(_:)), "a"),
            .separator(),
            submenuItem(L("Find"), [
                item(L("Find…"), #selector(GridViewController.showFindBar(_:)), "f"),
                item(L("Find and Replace…"), #selector(GridViewController.showFindAndReplaceBar(_:)), "f", [.command, .option]),
                item(L("Find Next"), #selector(GridViewController.findNextMatch(_:)), "g"),
                item(L("Find Previous"), #selector(GridViewController.findPreviousMatch(_:)), "G"),
            ]),
        ])
    }

    private static func makeTableMenu() -> NSMenu {
        menu([
            item(L("Edit Cell"), #selector(GridViewController.editCell(_:))),
            item(L("Edit Cell in Window…"), #selector(GridViewController.editCellInWindow(_:)), "e", [.command, .option]),
            item(L("Fill Down"), #selector(GridViewController.fillDown(_:)), "d"),
            .separator(),
            item(L("Insert Row Above"), #selector(GridViewController.insertRowAbove(_:)), key(NSUpArrowFunctionKey), [.command, .option]),
            item(L("Insert Row Below"), #selector(GridViewController.insertRowBelow(_:)), key(NSDownArrowFunctionKey), [.command, .option]),
            item(L("Delete Rows"), #selector(GridViewController.deleteRows(_:)), "\u{8}"),
            .separator(),
            item(L("Insert Column Left"), #selector(GridViewController.insertColumnLeft(_:)), key(NSLeftArrowFunctionKey), [.command, .option]),
            item(L("Insert Column Right"), #selector(GridViewController.insertColumnRight(_:)), key(NSRightArrowFunctionKey), [.command, .option]),
            item(L("Delete Column"), #selector(GridViewController.deleteColumn(_:)), "\u{8}", [.command, .option]),
            item(L("Rename Column…"), #selector(GridViewController.renameColumn(_:))),
            .separator(),
            item(L("Sort Ascending"), #selector(GridViewController.sortAscending(_:))),
            item(L("Sort Descending"), #selector(GridViewController.sortDescending(_:))),
            item(L("Clear Sort"), #selector(GridViewController.clearSort(_:))),
            item(L("Reorder Rows in File"), #selector(GridViewController.applySortToFile(_:))),
            .separator(),
            item(L("Show Filter"), #selector(GridViewController.toggleFilterBar(_:)), "l", [.command, .shift]),
            item(L("Clear Filter"), #selector(GridViewController.clearFilter(_:))),
            .separator(),
            item(L("First Row Is Header"), #selector(GridViewController.toggleHeaderRow(_:))),
        ])
    }

    private static func makeViewMenu() -> NSMenu {
        menu([
            item(L("Enter Full Screen"), #selector(NSWindow.toggleFullScreen(_:)), "f", [.command, .control]),
        ])
    }

    private static func makeWindowMenu() -> NSMenu {
        menu([
            item(L("Minimize"), #selector(NSWindow.performMiniaturize(_:)), "m"),
            item(L("Zoom"), #selector(NSWindow.performZoom(_:))),
            .separator(),
            item(L("Bring All to Front"), #selector(NSApplication.arrangeInFront(_:))),
        ])
    }

    private static func submenuItem(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.submenu = menu(items)
        item.submenu?.title = title
        return item
    }

    private static func menu(_ items: [NSMenuItem]) -> NSMenu {
        let menu = NSMenu()
        items.forEach(menu.addItem)
        return menu
    }

    private static func item(
        _ title: String,
        _ action: Selector?,
        _ keyEquivalent: String = "",
        _ modifiers: NSEvent.ModifierFlags = .command
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
        if !keyEquivalent.isEmpty {
            item.keyEquivalentModifierMask = modifiers
        }
        return item
    }

    private static func key(_ functionKey: Int) -> String {
        String(Character(UnicodeScalar(UInt32(functionKey))!))
    }
}

/// File ▸ Open Recent, filled from NSDocumentController's list each time it opens.
@MainActor
final class RecentDocumentsMenu: NSObject, NSMenuDelegate {
    static let shared = RecentDocumentsMenu()
    let menu = NSMenu()

    override private init() {
        super.init()
        menu.delegate = self
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let controller = NSDocumentController.shared
        for url in controller.recentDocumentURLs {
            let item = NSMenuItem(
                title: FileManager.default.displayName(atPath: url.path),
                action: #selector(openRecent(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = url
            item.image = NSWorkspace.shared.icon(forFile: url.path)
            item.image?.size = NSSize(width: 16, height: 16)
            menu.addItem(item)
        }
        if !controller.recentDocumentURLs.isEmpty {
            menu.addItem(.separator())
        }
        menu.addItem(
            NSMenuItem(
                title: L("Clear Menu"),
                action: #selector(NSDocumentController.clearRecentDocuments(_:)),
                keyEquivalent: ""
            )
        )
    }

    @objc private func openRecent(_ sender: NSMenuItem) {
        guard let url = sender.representedObject as? URL else { return }
        NSDocumentController.shared.openDocument(withContentsOf: url, display: true) { _, _, error in
            if let error {
                NSApp.presentError(error)
            }
        }
    }
}
