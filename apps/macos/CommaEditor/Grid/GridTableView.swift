import AppKit

@MainActor
protocol GridTableViewInteraction: AnyObject {
    /// Returns `true` if the key press was handled.
    func gridTableView(_ tableView: GridTableView, handleKeyDown event: NSEvent) -> Bool
    /// `tableColumn` is the index in `tableColumns`; 0 is the row number column.
    func gridTableView(_ tableView: GridTableView, mouseDownAtRow row: Int, tableColumn: Int, extend: Bool, clickCount: Int)
    func gridTableView(_ tableView: GridTableView, mouseDraggedToRow row: Int, tableColumn: Int)
    func gridTableViewSelectAll(_ tableView: GridTableView)
}

/// NSTableView that draws and virtualizes the cells while the grid controller owns selection:
/// clicks, drags and keys are forwarded instead of selecting rows.
final class GridTableView: NSTableView {
    weak var interaction: GridTableViewInteraction?

    override func keyDown(with event: NSEvent) {
        if interaction?.gridTableView(self, handleKeyDown: event) == true {
            return
        }
        super.keyDown(with: event)
    }

    override func mouseDown(with event: NSEvent) {
        guard let interaction else { return super.mouseDown(with: event) }
        window?.makeFirstResponder(self)
        let point = convert(event.locationInWindow, from: nil)
        let row = row(at: point)
        let column = column(at: point)
        guard row >= 0, column >= 0 else { return }
        interaction.gridTableView(
            self,
            mouseDownAtRow: row,
            tableColumn: column,
            extend: event.modifierFlags.contains(.shift),
            clickCount: event.clickCount
        )
        guard event.clickCount == 1 else { return }

        // Drag to extend the selection, scrolling when the pointer leaves the view.
        while let next = window?.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]),
              next.type == .leftMouseDragged {
            autoscroll(with: next)
            let point = convert(next.locationInWindow, from: nil)
            interaction.gridTableView(self, mouseDraggedToRow: nearestRow(to: point), tableColumn: nearestColumn(to: point))
        }
    }

    override func selectAll(_ sender: Any?) {
        guard let interaction else { return super.selectAll(sender) }
        interaction.gridTableViewSelectAll(self)
    }

    /// Cells are edited by double-click, Return or typing, never by a click on its own.
    override func validateProposedFirstResponder(_ responder: NSResponder, for event: NSEvent?) -> Bool {
        if responder is NSTextField, event?.type == .leftMouseDown {
            return false
        }
        return super.validateProposedFirstResponder(responder, for: event)
    }

    private func nearestRow(to point: NSPoint) -> Int {
        let row = row(at: point)
        if row >= 0 { return row }
        return point.y < 0 ? 0 : max(0, numberOfRows - 1)
    }

    private func nearestColumn(to point: NSPoint) -> Int {
        let column = column(at: point)
        if column >= 0 { return column }
        return point.x < 0 ? 0 : max(0, numberOfColumns - 1)
    }
}
