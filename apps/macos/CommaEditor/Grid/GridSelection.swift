/// A cell of the grid: a visible row and a data column.
struct GridCell: Equatable {
    var row: Int
    var column: Int
}

/// A rectangle of cells between the anchor (where selecting started) and the cursor (the
/// active cell, which moves with the keyboard and is edited on Return).
struct GridSelection: Equatable {
    var anchor: GridCell
    var cursor: GridCell

    init(_ cell: GridCell) {
        anchor = cell
        cursor = cell
    }

    init(anchor: GridCell, cursor: GridCell) {
        self.anchor = anchor
        self.cursor = cursor
    }

    var rows: ClosedRange<Int> { min(anchor.row, cursor.row)...max(anchor.row, cursor.row) }
    var columns: ClosedRange<Int> { min(anchor.column, cursor.column)...max(anchor.column, cursor.column) }
    var isSingleCell: Bool { anchor == cursor }
    var cellCount: Int { rows.count * columns.count }

    func contains(row: Int, column: Int) -> Bool {
        rows.contains(row) && columns.contains(column)
    }

    var range: CellRange {
        CellRange(
            top: UInt64(rows.lowerBound),
            left: UInt64(columns.lowerBound),
            bottom: UInt64(rows.upperBound),
            right: UInt64(columns.upperBound)
        )
    }

    /// Keeps both corners inside a grid of `rowCount` × `columnCount`; `nil` if it is empty.
    func clamped(rowCount: Int, columnCount: Int) -> GridSelection? {
        guard rowCount > 0, columnCount > 0 else { return nil }
        let clamp = { (cell: GridCell) in
            GridCell(row: min(max(0, cell.row), rowCount - 1), column: min(max(0, cell.column), columnCount - 1))
        }
        return GridSelection(anchor: clamp(anchor), cursor: clamp(cursor))
    }
}
