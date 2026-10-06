import AppKit

@MainActor
protocol GridTableViewKeyHandler: AnyObject {
    /// Returns `true` if the key press was handled.
    func gridTableView(_ tableView: GridTableView, handleKeyDown event: NSEvent) -> Bool
}

final class GridTableView: NSTableView {
    weak var keyHandler: GridTableViewKeyHandler?

    override func keyDown(with event: NSEvent) {
        if keyHandler?.gridTableView(self, handleKeyDown: event) == true {
            return
        }
        super.keyDown(with: event)
    }

    /// Cells are edited by double-click, Return or typing, not by clicking a selected row.
    override func validateProposedFirstResponder(_ responder: NSResponder, for event: NSEvent?) -> Bool {
        if responder is NSTextField, event?.type == .leftMouseDown {
            return false
        }
        return super.validateProposedFirstResponder(responder, for: event)
    }
}
