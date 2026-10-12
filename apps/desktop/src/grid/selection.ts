// The selection of the grid, as GridSelection.swift in the macOS app: a rectangle of visible
// rows and columns from the anchor, where it started, to the cursor, the active cell.

import type { CellRange } from "../lib/api";

export interface Cell {
  row: number;
  column: number;
}

export class Selection {
  constructor(
    readonly anchor: Cell,
    readonly cursor: Cell,
  ) {}

  static at(cell: Cell): Selection {
    return new Selection(cell, cell);
  }

  get range(): CellRange {
    return {
      top: Math.min(this.anchor.row, this.cursor.row),
      bottom: Math.max(this.anchor.row, this.cursor.row),
      left: Math.min(this.anchor.column, this.cursor.column),
      right: Math.max(this.anchor.column, this.cursor.column),
    };
  }

  get single(): boolean {
    return this.anchor.row === this.cursor.row && this.anchor.column === this.cursor.column;
  }

  get rowCount(): number {
    const range = this.range;
    return range.bottom - range.top + 1;
  }

  get columnCount(): number {
    const range = this.range;
    return range.right - range.left + 1;
  }

  contains(row: number, column: number): boolean {
    const { top, bottom, left, right } = this.range;
    return row >= top && row <= bottom && column >= left && column <= right;
  }

  /** The cursor moved to `cell`; with `extend`, the anchor stays. */
  moved(cell: Cell, extend: boolean): Selection {
    return extend ? new Selection(this.anchor, cell) : Selection.at(cell);
  }

  /** Moved by `rows`, as when the first row stops or starts being a row of the table. */
  shifted(rows: number): Selection {
    const shift = (cell: Cell) => ({ row: Math.max(0, cell.row + rows), column: cell.column });
    return new Selection(shift(this.anchor), shift(this.cursor));
  }

  /** Kept within a table of `rows` × `columns`; `null` when the table is empty. */
  clamped(rows: number, columns: number): Selection | null {
    if (rows === 0 || columns === 0) return null;
    const clamp = (cell: Cell) => ({ row: Math.min(cell.row, rows - 1), column: Math.min(cell.column, columns - 1) });
    return new Selection(clamp(this.anchor), clamp(this.cursor));
  }

  equals(other: Selection | null): boolean {
    return (
      other !== null &&
      other.anchor.row === this.anchor.row &&
      other.anchor.column === this.anchor.column &&
      other.cursor.row === this.cursor.row &&
      other.cursor.column === this.cursor.column
    );
  }
}
