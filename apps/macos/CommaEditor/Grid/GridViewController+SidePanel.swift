import AppKit

// The panel at the right of the table shows one of: the column profile, the file repair or
// the record card.

extension GridViewController {
    enum SidePanel {
        case profile, repair, card

        var width: CGFloat {
            switch self {
            case .profile: 280
            case .repair: 320
            case .card: 360
            }
        }
    }

    /// Shows one of the panels at the right of the table, or hides the panel.
    func showSidePanel(_ panel: SidePanel?) {
        let previous = shownSidePanel
        guard panel != previous else { return }
        shownSidePanel = panel
        sidePanel.isHidden = panel == nil
        profileHost.isHidden = panel != .profile
        repairHost.isHidden = panel != .repair
        cardHost.isHidden = panel != .card
        sidePanelWidth.constant = panel?.width ?? 0
        // The table got narrower or wider: keep the active cell in view.
        view.layoutSubtreeIfNeeded()
        if let cursor = selection?.cursor {
            scrollToCell(row: cursor.row, column: cursor.column)
        }
        if previous == .profile {
            profiledColumn = nil
            profileModel.profile = nil
            reloadTables()
        }
        switch panel {
        case .profile?:
            configureProfileActions()
            refreshProfile()
        case .repair?:
            repairCursorMoved()
        case .card?:
            refreshCard()
        case nil:
            view.window?.makeFirstResponder(tableView)
        }
    }
}
