import AppKit

extension NSToolbarItem.Identifier {
    static let insertRow = Self("insertRow")
    static let insertColumn = Self("insertColumn")
    static let deleteRows = Self("deleteRows")
    static let sort = Self("sort")
    static let filter = Self("filter")
    static let data = Self("data")
    static let export = Self("export")
    static let search = Self("search")
    static let profile = Self("profile")
    static let repair = Self("repair")
}

/// The window toolbar: row and column commands, sort, filter, data operations, export and
/// search. Buttons send their actions down the responder chain to the grid; people can
/// rearrange the toolbar or show labels with View ▸ Customize Toolbar.
@MainActor
final class DocumentToolbar: NSObject, NSToolbarDelegate, NSSearchFieldDelegate {
    let toolbar = NSToolbar(identifier: "DocumentToolbar")
    weak var grid: GridViewController?
    private(set) var searchItem: NSSearchToolbarItem?
    private weak var sortItem: NSToolbarItem?
    private weak var filterItem: NSToolbarItem?
    private var sortActive = false
    private var filterActive = false

    override init() {
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = true
        toolbar.autosavesConfiguration = true
    }

    /// Search text, when the search field is in the toolbar.
    var searchField: NSSearchField? {
        guard let searchItem, toolbar.isVisible, toolbar.items.contains(searchItem) else { return nil }
        return searchItem.searchField
    }

    func beginSearch() -> Bool {
        guard let searchItem, searchField != nil else { return false }
        searchItem.beginSearchInteraction()
        return true
    }

    /// Filled icons show that rows are sorted or filtered.
    func update(sortActive: Bool, filterActive: Bool) {
        guard sortActive != self.sortActive || filterActive != self.filterActive else { return }
        self.sortActive = sortActive
        self.filterActive = filterActive
        sortItem?.image = Self.symbol(sortActive ? "arrow.up.arrow.down.circle.fill" : "arrow.up.arrow.down.circle")
        filterItem?.image = Self.symbol(
            filterActive ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle"
        )
    }

    // MARK: NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.insertRow, .insertColumn, .deleteRows, .space, .sort, .filter, .data, .export, .profile, .repair, .flexibleSpace, .search]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.insertRow, .insertColumn, .deleteRows, .sort, .filter, .data, .export, .profile, .repair, .search, .space, .flexibleSpace]
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        switch identifier {
        case .insertRow:
            return button(identifier, L("Insert Row"), "rectangle.bottomhalf.inset.filled",
                          #selector(GridViewController.insertRowBelow(_:)), tip: L("Insert a row below (⌥⌘↓)"))
        case .insertColumn:
            return button(identifier, L("Insert Column"), "rectangle.righthalf.inset.filled",
                          #selector(GridViewController.insertColumnRight(_:)), tip: L("Insert a column to the right (⌥⌘→)"))
        case .deleteRows:
            return button(identifier, L("Delete Rows"), "trash",
                          #selector(GridViewController.deleteRows(_:)), tip: L("Delete the selected rows (⌘⌫)"))
        case .sort:
            let item = menuButton(identifier, L("Sort"), sortActive ? "arrow.up.arrow.down.circle.fill" : "arrow.up.arrow.down.circle",
                                  MainMenu.makeToolbarSortMenu(), tip: L("Sort by the active column"))
            sortItem = item
            return item
        case .filter:
            let item = button(identifier, L("Filter"),
                              filterActive ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle",
                              #selector(GridViewController.toggleFilterBar(_:)), tip: L("Show or hide the filter (⇧⌘L)"))
            filterItem = item
            return item
        case .data:
            return menuButton(identifier, L("Data"), "wand.and.stars", MainMenu.makeDataMenu(),
                              tip: L("Clean up text, fill empty cells, split and merge columns"))
        case .export:
            return menuButton(identifier, L("Export"), "square.and.arrow.up", MainMenu.makeToolbarExportMenu(),
                              tip: L("Export for Excel, JSON, Markdown, HTML, SQL"))
        case .profile:
            return button(identifier, L("Profile"), "chart.bar.xaxis",
                          #selector(GridViewController.toggleProfilePanel(_:)), tip: L("Column profile: type, statistics, problems (⌥⌘P)"))
        case .repair:
            return button(identifier, L("Repair"), "bandage",
                          #selector(GridViewController.toggleRepairPanel(_:)), tip: L("Find and fix broken rows (⌥⌘R)"))
        case .search:
            let item = NSSearchToolbarItem(itemIdentifier: identifier)
            item.label = L("Find")
            item.paletteLabel = L("Find")
            item.toolTip = L("Find in the table (⌘F)")
            item.searchField.placeholderString = L("Find")
            item.searchField.delegate = self
            item.preferredWidthForSearchField = 200
            searchItem = item
            return item
        default:
            return nil
        }
    }

    // MARK: Search field

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSSearchField else { return }
        grid?.toolbarSearchChanged(field.stringValue)
    }

    /// Return finds the next match, Shift-Return the previous one, Escape gives the table focus.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.insertNewline(_:)):
            let backwards = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
            backwards ? grid?.findPreviousMatch(nil) : grid?.findNextMatch(nil)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            (control as? NSSearchField)?.stringValue = ""
            grid?.toolbarSearchChanged("")
            grid?.focusTable()
            return true
        default:
            return false
        }
    }

    // MARK: Items

    private func button(_ identifier: NSToolbarItem.Identifier, _ label: String, _ symbol: String,
                        _ action: Selector, tip: String) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = label
        item.paletteLabel = label
        item.toolTip = tip
        item.image = Self.symbol(symbol, label)
        item.action = action
        item.isBordered = true
        return item
    }

    private func menuButton(_ identifier: NSToolbarItem.Identifier, _ label: String, _ symbol: String,
                            _ menu: NSMenu, tip: String) -> NSMenuToolbarItem {
        let item = NSMenuToolbarItem(itemIdentifier: identifier)
        item.label = label
        item.paletteLabel = label
        item.toolTip = tip
        item.image = Self.symbol(symbol, label)
        item.menu = menu
        item.showsIndicator = true
        return item
    }

    private static func symbol(_ name: String, _ description: String? = nil) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: description)
    }
}
