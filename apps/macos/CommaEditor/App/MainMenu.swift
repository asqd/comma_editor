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
            (L("Data"), makeDataMenu()),
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
            submenuItem(L("Export As"), exportAsItems()),
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
            submenuItem(L("Copy As"), [
                tagged(item(L("Markdown"), #selector(GridViewController.copyAs(_:))), 1),
                tagged(item(L("JSON"), #selector(GridViewController.copyAs(_:))), 0),
                tagged(item(L("HTML"), #selector(GridViewController.copyAs(_:))), 2),
                tagged(item(L("SQL"), #selector(GridViewController.copyAs(_:))), 3),
            ]),
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
            item(L("Move Rows Up"), #selector(GridViewController.moveRowsUp(_:)), key(NSUpArrowFunctionKey), [.command, .control]),
            item(L("Move Rows Down"), #selector(GridViewController.moveRowsDown(_:)), key(NSDownArrowFunctionKey), [.command, .control]),
            .separator(),
            item(L("Insert Column Left"), #selector(GridViewController.insertColumnLeft(_:)), key(NSLeftArrowFunctionKey), [.command, .option]),
            item(L("Insert Column Right"), #selector(GridViewController.insertColumnRight(_:)), key(NSRightArrowFunctionKey), [.command, .option]),
            item(L("Delete Column"), #selector(GridViewController.deleteColumn(_:)), "\u{8}", [.command, .option]),
            item(L("Rename Column…"), #selector(GridViewController.renameColumn(_:))),
            item(L("Move Column Left"), #selector(GridViewController.moveColumnLeft(_:)), key(NSLeftArrowFunctionKey), [.command, .control]),
            item(L("Move Column Right"), #selector(GridViewController.moveColumnRight(_:)), key(NSRightArrowFunctionKey), [.command, .control]),
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

    /// File ▸ Export As items; also used by the toolbar's export menu.
    static func exportAsItems() -> [NSMenuItem] {
        [
            tagged(item(L("JSON…"), #selector(GridViewController.exportAs(_:))), 0),
            tagged(item(L("Markdown…"), #selector(GridViewController.exportAs(_:))), 1),
            tagged(item(L("HTML…"), #selector(GridViewController.exportAs(_:))), 2),
            tagged(item(L("SQL…"), #selector(GridViewController.exportAs(_:))), 3),
            tagged(item(L("Excel Workbook (XLSX)…"), #selector(GridViewController.exportAs(_:))), 4),
        ]
    }

    /// Menus for the toolbar: a menu can belong to one place only, so each call builds a new one.
    static func makeToolbarExportMenu() -> NSMenu {
        menu([
            item(L("Export for Excel…"), #selector(CSVDocument.exportForExcel(_:))),
            item(L("Export…"), #selector(NSDocument.saveTo(_:))),
            .separator(),
        ] + exportAsItems())
    }

    static func makeToolbarSortMenu() -> NSMenu {
        menu([
            item(L("Sort Ascending"), #selector(GridViewController.sortAscending(_:))),
            item(L("Sort Descending"), #selector(GridViewController.sortDescending(_:))),
            item(L("Clear Sort"), #selector(GridViewController.clearSort(_:))),
            .separator(),
            item(L("Reorder Rows in File"), #selector(GridViewController.applySortToFile(_:))),
        ])
    }

    static func makeDataMenu() -> NSMenu {
        menu([
            submenuItem(L("Text"), [
                item(L("Trim Spaces"), #selector(GridViewController.trimSpaces(_:))),
                item(L("Remove Extra Spaces"), #selector(GridViewController.collapseSpaces(_:))),
                .separator(),
                item(L("UPPERCASE"), #selector(GridViewController.makeUppercase(_:))),
                item(L("lowercase"), #selector(GridViewController.makeLowercase(_:))),
                item(L("Capitalize Each Word"), #selector(GridViewController.capitalizeWords(_:))),
            ]),
            item(L("Fill Empty Cells from Above"), #selector(GridViewController.fillEmptyFromAbove(_:))),
            item(L("Fill Empty Cells…"), #selector(GridViewController.fillEmptyWithValue(_:))),
            .separator(),
            item(L("Split Column…"), #selector(GridViewController.splitColumn(_:))),
            item(L("Merge Columns…"), #selector(GridViewController.mergeColumns(_:))),
            item(L("Computed Column…"), #selector(GridViewController.computedColumn(_:)), "=", [.command, .option]),
            .separator(),
            item(L("Remove Duplicate Rows…"), #selector(GridViewController.removeDuplicateRows(_:))),
        ])
    }

    private static func makeViewMenu() -> NSMenu {
        menu([
            item(L("Show Column Profile"), #selector(GridViewController.toggleProfilePanel(_:)), "p", [.command, .option]),
            item(L("Show File Repair"), #selector(GridViewController.toggleRepairPanel(_:)), "r", [.command, .option]),
            item(L("Show Record Card"), #selector(GridViewController.toggleRecordCard(_:)), "i", [.command, .option]),
            item(L("Previous Record"), #selector(GridViewController.previousRecord(_:)), "["),
            item(L("Next Record"), #selector(GridViewController.nextRecord(_:)), "]"),
            .separator(),
            item(L("Freeze Columns Through This One"), #selector(GridViewController.freezeColumns(_:))),
            item(L("Unfreeze Columns"), #selector(GridViewController.unfreezeColumns(_:))),
            item(L("Hide Column"), #selector(GridViewController.hideColumns(_:))),
            item(L("Show All Columns"), #selector(GridViewController.showAllColumns(_:))),
            item(L("Fit Column Width"), #selector(GridViewController.autofitColumns(_:)), "0", [.command, .option]),
            item(L("Show Invisible Characters"), #selector(GridViewController.toggleInvisibles(_:)), "i", [.command, .shift]),
            submenuItem(L("Text Wrapping"), [
                tagged(item(L("One Line"), #selector(GridViewController.setLinesPerRow(_:))), 1),
                tagged(item(L("Up to 2 Lines"), #selector(GridViewController.setLinesPerRow(_:))), 2),
                tagged(item(L("Up to 3 Lines"), #selector(GridViewController.setLinesPerRow(_:))), 3),
                tagged(item(L("Up to 5 Lines"), #selector(GridViewController.setLinesPerRow(_:))), 5),
            ]),
            .separator(),
            item(L("Show Toolbar"), #selector(NSWindow.toggleToolbarShown(_:)), "t", [.command, .option]),
            item(L("Customize Toolbar…"), #selector(NSWindow.runToolbarCustomizationPalette(_:))),
            .separator(),
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

    private static func tagged(_ item: NSMenuItem, _ tag: Int) -> NSMenuItem {
        item.tag = tag
        return item
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
