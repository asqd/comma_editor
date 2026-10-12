import AppKit

@MainActor
protocol GridHeaderViewDelegate: AnyObject {
    /// The filter icon of a column header was clicked; `tableColumn` indexes `tableColumns`.
    func gridHeaderView(_ headerView: GridHeaderView, didClickFilterOf tableColumn: Int)
}

/// Column headers with sort and filter icons. The icons show on the column under the pointer,
/// and stay on columns that are sorted or filtered. A click on the filter icon opens the
/// filter for that column; any other click sorts, as before.
final class GridHeaderView: NSTableHeaderView {
    weak var gridDelegate: GridHeaderViewDelegate?
    private var trackingArea: NSTrackingArea?

    private var hoveredColumn = -1 {
        didSet {
            guard hoveredColumn != oldValue, let tableView else { return }
            for (index, column) in tableView.tableColumns.enumerated() {
                (column.headerCell as? GridHeaderCell)?.isHovered = index == hoveredColumn
            }
            needsDisplay = true
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let trackingArea {
            removeTrackingArea(trackingArea)
        }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseMoved(with event: NSEvent) {
        hoveredColumn = column(at: convert(event.locationInWindow, from: nil))
    }

    override func mouseExited(with event: NSEvent) {
        hoveredColumn = -1
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let index = column(at: point)
        if index >= 0, let tableView,
           let cell = tableView.tableColumns[index].headerCell as? GridHeaderCell, cell.showsFilterIcon,
           GridHeaderCell.filterRect(in: headerRect(ofColumn: index)).insetBy(dx: -3, dy: -3).contains(point) {
            gridDelegate?.gridHeaderView(self, didClickFilterOf: index)
            return
        }
        super.mouseDown(with: event)
    }
}

/// A column header that draws sort and filter icons at its right edge.
final class GridHeaderCell: NSTableHeaderCell {
    enum Sort: Equatable {
        case none
        case ascending
        case descending
    }

    var sort = Sort.none
    var isFiltered = false
    var isHovered = false

    var showsSortIcon: Bool { isHovered || sort != .none }
    var showsFilterIcon: Bool { isHovered || isFiltered }
    var showsIcons: Bool { showsSortIcon || showsFilterIcon }

    private static let iconWidth: CGFloat = 16
    private static let gap: CGFloat = 3

    /// VoiceOver reads the column with its sort and filter, which the icons only show.
    override func accessibilityLabel() -> String? {
        var parts = [stringValue]
        switch sort {
        case .ascending: parts.append(L("sorted ascending"))
        case .descending: parts.append(L("sorted descending"))
        case .none: break
        }
        if isFiltered {
            parts.append(L("filtered"))
        }
        return parts.joined(separator: ", ")
    }

    static func sortRect(in frame: NSRect) -> NSRect {
        NSRect(x: frame.maxX - iconWidth - 5, y: frame.minY, width: iconWidth, height: frame.height)
    }

    static func filterRect(in frame: NSRect) -> NSRect {
        NSRect(x: sortRect(in: frame).minX - gap - iconWidth, y: frame.minY, width: iconWidth, height: frame.height)
    }

    override func drawInterior(withFrame cellFrame: NSRect, in controlView: NSView) {
        // The header view also draws the empty area right of the last column with this cell.
        guard showsIcons, !isFiller(cellFrame, in: controlView) else {
            return super.drawInterior(withFrame: cellFrame, in: controlView)
        }
        let iconsStart = showsFilterIcon ? Self.filterRect(in: cellFrame).minX : Self.sortRect(in: cellFrame).minX
        var titleFrame = cellFrame
        titleFrame.size.width = max(0, iconsStart - cellFrame.minX - 2)
        super.drawInterior(withFrame: titleFrame, in: controlView)

        let accent = NSColor.controlAccentColor
        let quiet = NSColor.tertiaryLabelColor
        switch sort {
        case .ascending: Self.draw("chevron.up", accent, in: Self.sortRect(in: cellFrame))
        case .descending: Self.draw("chevron.down", accent, in: Self.sortRect(in: cellFrame))
        case .none where isHovered: Self.draw("arrow.up.arrow.down", quiet, in: Self.sortRect(in: cellFrame))
        case .none: break
        }
        if showsFilterIcon {
            Self.draw("line.3.horizontal.decrease.circle", isFiltered ? accent : quiet, in: Self.filterRect(in: cellFrame))
        }
    }

    private func isFiller(_ frame: NSRect, in controlView: NSView) -> Bool {
        guard let tableView = (controlView as? NSTableHeaderView)?.tableView,
              tableView.numberOfColumns > 0 else { return false }
        return frame.minX >= tableView.rect(ofColumn: tableView.numberOfColumns - 1).maxX - 1
    }

    private static func draw(_ symbol: String, _ color: NSColor, in rect: NSRect) {
        let configuration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        guard let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration) else { return }
        let size = image.size
        let target = NSRect(
            x: rect.midX - size.width / 2,
            y: rect.midY - size.height / 2,
            width: size.width,
            height: size.height
        )
        image.draw(in: target, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }
}
