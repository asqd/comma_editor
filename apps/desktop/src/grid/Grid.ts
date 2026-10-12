// The table: a canvas that draws only the cells in view, over a scroller whose scrollbars stand
// for the whole table. Rows come from the core in blocks, as the view reaches them.
//
// A table of 60 million rows is 1.4 billion pixels tall, far past the largest element a
// browser lays out (about 33 million px in Chromium, less in WebKit). So the scroll position
// lives here, in pixels of the table, and the scroller's range is that position scaled down:
// wheel and keys move the table by their own pixels, the scrollbar thumb by its share.
//
// The view is four parts, as frozen panes make it: the frozen rows and columns stay, the rest
// scrolls. Positions below are of the canvas unless said otherwise.

import type { ViewSpec } from "../lib/api";
import { Selection, type Cell } from "./selection";
import { Stats } from "./stats";
import { invisibleRuns, plainRuns, wrap, type Run } from "./text";

/** The rows of a table, as the core shows them with the current sort and filters. */
export interface TableSource {
  /** Data rows, without the header. */
  readonly rows: number;
  readonly columns: number;
  /** Column titles; empty when the file has no header row. */
  readonly header: readonly string[];
  /** Up to `count` rows from `start`, fewer at the end, with the numbers they show. */
  fetch(start: number, count: number): Promise<{ rows: string[][]; numbers: number[] }>;
}

/**
 * Cells of one column marked as problems while the column profile is open: a mark per visible
 * row, 0 for none, and what each mark means for the cell's tooltip.
 */
export interface Marker {
  column: number;
  fetch(start: number, count: number): Promise<number[]>;
  describe(mark: number): string;
}

/** Row heights for one line and each line more (22 and 16 in the macOS app, plus its grid line). */
const LINE_ROW = 24;
const MORE_LINE = 16;
const HEADER_HEIGHT = 28;
const PADDING = 8;
/** Widths as the macOS app has them: every column starts at 140, none narrower than 36. */
const DEFAULT_WIDTH = 140;
const MIN_WIDTH = 36;
const MAX_FIT = 480;
/** Room a fitted column keeps beside its title for the sort and filter icons. */
const ICON_ROOM = 44;
/** Rows measured to fit a column, besides those in view. */
const FIT_ROWS = 300;
/** The icons at the right of a column's title, as GridHeaderView draws them. */
const ICON = 16;
const SORT_ICON_INSET = 5;
const FILTER_ICON_GAP = 3;
/** Pixels either side of a column's right edge that grab it to resize. */
const RESIZE_GRIP = 4;
/** Pixels the pointer moves before a press on a title or a row number becomes a drag. */
const DRAG_START = 4;
/** Rows per request to the core. */
const BLOCK = 128;
/** Blocks kept, about 32 thousand rows. */
const MAX_BLOCKS = 256;
const MAX_IN_FLIGHT = 3;
/** Pixels the scrollbars stand for at most; beyond that the scroller's range is scaled. */
const MAX_SCROLL_RANGE = 4_000_000;
const FONT = '13px system-ui, -apple-system, "Segoe UI", Ubuntu, Cantarell, sans-serif';
const HEADER_FONT = `600 ${FONT}`;
const NUMBER_FONT = '11px ui-monospace, "SF Mono", "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace';
/** Pixels from an edge where dragging a selection scrolls, and the most it scrolls a frame. */
const AUTOSCROLL_EDGE = 24;
const AUTOSCROLL_STEP = 40;
const mac = navigator.userAgent.includes("Mac");

interface Colors {
  background: string;
  alternate: string;
  text: string;
  muted: string;
  quiet: string;
  line: string;
  header: string;
  headerText: string;
  placeholder: string;
  selection: string;
  cursor: string;
  selectedHeader: string;
  issue: string;
  mark: string;
  markBackground: string;
  missing: string;
}

/** What is under a point of the grid. */
type Hit =
  | { area: "cell"; row: number; column: number }
  | { area: "header"; column: number }
  | { area: "gutter"; row: number }
  | { area: "corner" | "none" };

/** A press of the pointer, and what it becomes as the pointer moves. */
type Press =
  | { kind: "select"; rows: boolean; x: number; y: number; frame: number }
  | { kind: "resize"; column: number; x: number; width: number }
  | { kind: "title"; column: number; x: number; y: number; shift: boolean; moving: boolean; target: number }
  | { kind: "rows"; x: number; y: number; moving: boolean; target: number; frame: number };

/** Column widths and hidden columns, kept apart from the table to restore with undo. */
export interface ColumnLayout {
  widths: number[];
  hidden: number[];
}

export class Grid {
  readonly stats = new Stats();
  /** Called after each frame with whether every row in view had its data. */
  onFrame: ((complete: boolean) => void) | null = null;
  /** The selection changed. */
  onSelect: ((selection: Selection | null) => void) | null = null;
  /** A double click on a cell: edit it. */
  onActivate: ((cell: Cell) => void) | null = null;
  /** A click on a column's title, with Shift or not. */
  onHeader: ((column: number, shift: boolean) => void) | null = null;
  /** A click on the filter icon of a column's title. */
  onFilterIcon: ((column: number) => void) | null = null;
  /** Run after every frame, e.g. to keep an editor over its cell. */
  readonly afterDraw: (() => void)[] = [];
  /** Text typed into the grid, composed by an input method or not: it starts editing. */
  onType: ((text: string) => void) | null = null;
  /** Copy or cut from the menu or the keyboard. */
  onCopy: ((cut: boolean) => void) | null = null;
  /** Text pasted into the grid. */
  onPaste: ((text: string) => void) | null = null;
  /** A right click on a cell or a column's title, where the pointer is. */
  onMenu: ((target: { area: "cell"; cell: Cell } | { area: "header"; column: number }, x: number, y: number) => void) | null = null;
  /** Selected whole rows dragged by their numbers, to go before visible row `before`. */
  onMoveRows: ((before: number) => void) | null = null;
  /** A column dragged by its title to another place. */
  onMoveColumn: ((from: number, to: number) => void) | null = null;
  /** Words for a missing field's tooltip. */
  missingText = "";
  /** Whether rows can be dragged now: not while sorted. */
  canMoveRows: () => boolean = () => false;

  private readonly scroller: HTMLDivElement;
  /**
   * Where the grid's keys go: a field out of sight at the active cell. Being editable, it gets
   * copy, cut and paste in every WebView, from the keyboard and the menu, and text from input
   * methods, which a canvas cannot.
   */
  private readonly keys: HTMLTextAreaElement;
  private readonly spacer: HTMLDivElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly context: CanvasRenderingContext2D;
  private colors: Colors;
  private dpr = 1;
  private width = 0;
  private height = 0;

  private source: TableSource | null = null;
  /** Bumps with each table, so that rows of the previous one are dropped when they arrive. */
  private generation = 0;
  /** Scroll position of the scrolling rows and columns, past the frozen ones. */
  private top = 0;
  private left = 0;
  /** Where the scroller was last put, to tell its own scrolling from ours. */
  private placed = { top: 0, left: 0 };
  private ratioY = 1;
  private ratioX = 1;

  private gutter = 48;
  private widths: number[] = [];
  private readonly hidden = new Set<number>();
  /** Left edge of each column from the first, hidden ones taking no room, and the total. */
  private edges: number[] = [0];
  private frozenColumnCount = 0;
  private frozenRowCount = 0;
  private lines = 1;
  private invisibles = false;

  private selected: Selection | null = null;
  /** Sorted columns: ascending or not, and their place among several keys. */
  private sorted = new Map<number, { ascending: boolean; order: number }>();
  private filtered = new Set<number | null>();
  /** The column whose title the pointer is over: its icons show. */
  private hovered: number | null = null;
  private press: Press | null = null;

  private marker: Marker | null = null;
  /** Marks of the marker's column, by block of rows. */
  private readonly marks = new Map<number, number[]>();
  /** Rows marked on their numbers, as broken rows are while the file has them. */
  private rowMarker: ((start: number, count: number) => Promise<number[]>) | null = null;
  private readonly rowMarks = new Map<number, number[]>();
  private readonly blocks = new Map<number, string[][]>();
  private readonly numbers = new Map<number, number[]>();
  private readonly pending = new Set<number>();
  private queue: number[] = [];
  private inFlight = 0;
  private frame = 0;

  constructor(private readonly host: HTMLElement) {
    this.scroller = document.createElement("div");
    this.scroller.className = "grid-scroller";
    this.keys = document.createElement("textarea");
    this.keys.className = "grid-keys";
    this.keys.autocomplete = "off";
    this.keys.spellcheck = false;
    this.keys.setAttribute("autocorrect", "off");
    this.keys.setAttribute("autocapitalize", "off");
    this.spacer = document.createElement("div");
    this.spacer.className = "grid-spacer";
    this.scroller.append(this.spacer);
    this.canvas = document.createElement("canvas");
    this.canvas.className = "grid-canvas";
    // The scroller lies over the canvas, transparent, so that overlay scrollbars stay visible.
    host.append(this.canvas, this.scroller, this.keys);
    const context = this.canvas.getContext("2d", { alpha: false });
    if (!context) throw new Error("no 2D canvas");
    this.context = context;
    this.colors = this.readColors();

    this.scroller.addEventListener("scroll", () => this.scrolled());
    this.scroller.addEventListener("wheel", (event) => this.wheel(event), { passive: false });
    this.keys.addEventListener("keydown", (event) => this.key(event));
    this.keys.addEventListener("input", (event) => {
      if (!(event as InputEvent).isComposing) this.typed();
    });
    this.keys.addEventListener("compositionend", () => this.typed());
    this.keys.addEventListener("focus", () => this.arm());
    this.keys.addEventListener("copy", (event) => this.clipboard(event, false));
    this.keys.addEventListener("cut", (event) => this.clipboard(event, true));
    this.keys.addEventListener("paste", (event) => {
      event.preventDefault();
      this.arm();
      this.onPaste?.(event.clipboardData?.getData("text/plain") ?? "");
    });
    this.scroller.addEventListener("pointerdown", (event) => this.pointerDown(event));
    this.scroller.addEventListener("pointermove", (event) => this.pointerMove(event));
    this.scroller.addEventListener("pointerup", (event) => this.pointerUp(event));
    this.scroller.addEventListener("dblclick", (event) => this.doubleClick(event));
    this.scroller.addEventListener("contextmenu", (event) => this.contextMenu(event));
    this.scroller.addEventListener("pointerleave", () => {
      if (this.hovered !== null) {
        this.hovered = null;
        this.invalidate();
      }
    });
    new ResizeObserver(() => this.resize()).observe(this.scroller);
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      this.colors = this.readColors();
      this.invalidate();
    });
    this.resize();
  }

  // The table and its layout

  /** Shows another table, from the top, every column 140 wide and shown, nothing frozen. */
  setSource(source: TableSource | null): void {
    this.source = source;
    this.dropRows();
    this.top = 0;
    this.left = 0;
    this.selected = null;
    this.widths = Array.from({ length: source?.columns ?? 0 }, () => DEFAULT_WIDTH);
    this.hidden.clear();
    this.frozenColumnCount = 0;
    this.frozenRowCount = 0;
    this.layout();
    this.invalidate();
    this.focus();
  }

  /** The rows changed: an edit, undo, sort or filter. Keeps the place and the layout. */
  reload(): void {
    this.dropRows();
    this.update();
  }

  /** The table grew or shrank: more rows were indexed, or columns came or went. */
  update(): void {
    const columns = this.source?.columns ?? 0;
    while (this.widths.length < columns) this.widths.push(DEFAULT_WIDTH);
    this.widths.length = columns;
    for (const column of this.hidden) if (column >= columns) this.hidden.delete(column);
    // The last block may have been cut short by the end of what was indexed then.
    for (const [index, rows] of this.blocks) if (rows.length < BLOCK) this.blocks.delete(index);
    this.layout();
    const clamped = this.selected?.clamped(this.source?.rows ?? 0, columns) ?? null;
    if (!(clamped?.equals(this.selected) ?? this.selected === null)) this.select(clamped, false);
    this.invalidate();
  }

  private dropRows(): void {
    this.generation += 1;
    this.blocks.clear();
    this.numbers.clear();
    this.marks.clear();
    this.rowMarks.clear();
    this.pending.clear();
    this.queue = [];
    this.inFlight = 0;
  }

  /** Shows which columns sort the rows and which are filtered. */
  setView(view: ViewSpec): void {
    this.sorted = new Map(view.sort.map((key, order) => [key.column, { ascending: key.ascending, order }]));
    this.filtered = new Set(view.filters.map((rule) => rule.column));
    this.invalidate();
  }

  /** Lines each row shows, from 1 to 5 (View ▸ Text Wrapping). */
  get linesPerRow(): number {
    return this.lines;
  }

  setLinesPerRow(lines: number): void {
    this.lines = lines;
    this.layout();
    if (this.selected) this.reveal(this.selected.cursor);
    this.invalidate();
  }

  get showsInvisibles(): boolean {
    return this.invisibles;
  }

  setInvisibles(shown: boolean): void {
    this.invisibles = shown;
    this.invalidate();
  }

  get frozenColumns(): number {
    return this.frozenColumnCount;
  }

  /** Freezes the first columns; at least one keeps scrolling. */
  freezeColumns(count: number): void {
    this.frozenColumnCount = Math.max(0, Math.min(count, this.widths.length - 1));
    this.left = 0;
    this.layout();
    if (this.selected) this.reveal(this.selected.cursor);
    this.invalidate();
  }

  get frozenRows(): number {
    return this.frozenRowCount;
  }

  /** Freezes the first shown rows, whatever they are under the sort; at least one scrolls. */
  freezeRows(count: number): void {
    this.frozenRowCount = Math.max(0, Math.min(count, (this.source?.rows ?? 0) - 1));
    this.top = 0;
    this.layout();
    if (this.selected) this.reveal(this.selected.cursor);
    this.invalidate();
  }

  get hiddenCount(): number {
    return this.hidden.size;
  }

  isHidden(column: number): boolean {
    return this.hidden.has(column);
  }

  hideColumns(columns: number[]): void {
    for (const column of columns) this.hidden.add(column);
    this.layout();
    this.invalidate();
  }

  showAllColumns(): void {
    this.hidden.clear();
    this.layout();
    this.invalidate();
  }

  /** The nearest shown column to `column`, looking right and then left; `null` without one. */
  shownColumn(column: number, direction: 1 | -1 = 1): number | null {
    for (let next = column; next >= 0 && next < this.widths.length; next += direction) if (!this.hidden.has(next)) return next;
    for (let next = column; next >= 0 && next < this.widths.length; next -= direction) if (!this.hidden.has(next)) return next;
    return null;
  }

  /** Widths and hidden columns, to restore after an undo moves columns back. */
  get columnLayout(): ColumnLayout {
    return { widths: [...this.widths], hidden: [...this.hidden] };
  }

  set columnLayout(layout: ColumnLayout) {
    this.widths = [...layout.widths];
    this.hidden.clear();
    for (const column of layout.hidden) this.hidden.add(column);
    this.update();
  }

  /** A column went in at `at`: widths and hidden columns after it move right. */
  insertColumn(at: number): void {
    this.widths.splice(at, 0, DEFAULT_WIDTH);
    this.shiftHidden((column) => (column >= at ? column + 1 : column));
    this.layout();
  }

  /** The column at `at` went: widths and hidden columns after it move left. */
  deleteColumn(at: number): void {
    this.widths.splice(at, 1);
    this.hidden.delete(at);
    this.shiftHidden((column) => (column > at ? column - 1 : column));
    this.layout();
  }

  /** A column moved: its width and hidden state go with it. */
  moveColumn(from: number, to: number): void {
    const [width] = this.widths.splice(from, 1);
    this.widths.splice(to, 0, width ?? DEFAULT_WIDTH);
    const wasHidden = this.hidden.delete(from);
    this.shiftHidden((column) => (column > from ? column - 1 : column));
    this.shiftHidden((column) => (column >= to ? column + 1 : column));
    if (wasHidden) this.hidden.add(to);
    this.layout();
  }

  private shiftHidden(move: (column: number) => number): void {
    const moved = [...this.hidden].map(move);
    this.hidden.clear();
    for (const column of moved) this.hidden.add(column);
  }

  /**
   * Fits columns to their titles and values, as Fit Column Width in the macOS app: the first
   * 300 rows and those in view are measured.
   */
  async fitColumns(columns: number[]): Promise<void> {
    const source = this.source;
    if (!source || columns.length === 0) return;
    const generation = this.generation;
    const { rows } = await source.fetch(0, FIT_ROWS);
    if (generation !== this.generation) return;
    const inView = this.rowsInView().flatMap(({ row }) => (this.row(row) ? [this.row(row)!] : []));
    for (const column of columns) {
      this.context.font = HEADER_FONT;
      let widest = this.context.measureText(this.title(column)).width + ICON_ROOM;
      this.context.font = FONT;
      for (const row of [...rows, ...inView]) {
        const value = row[column];
        if (!value) continue;
        const text = value.length > 200 ? value.slice(0, 200) : value;
        widest = Math.max(widest, this.context.measureText(text.replace(/\r\n|\r|\n/g, " ↵ ")).width + 2 * PADDING);
      }
      this.widths[column] = Math.min(MAX_FIT, Math.max(MIN_WIDTH, Math.ceil(widest)));
    }
    this.layout();
    this.invalidate();
  }

  // Markers

  /** Marks problem cells of a column, or stops marking them. */
  setMarker(marker: Marker | null): void {
    this.marker = marker;
    this.marks.clear();
    for (const block of this.blocks.keys()) this.fetchMarks(block, this.generation);
    this.invalidate();
  }

  /** Marks rows on their numbers, or stops marking them. */
  setRowMarker(fetch: ((start: number, count: number) => Promise<number[]>) | null): void {
    this.rowMarker = fetch;
    this.rowMarks.clear();
    for (const block of this.blocks.keys()) this.fetchRowMarks(block, this.generation);
    this.invalidate();
  }

  private fetchMarks(block: number, generation: number): void {
    const marker = this.marker;
    if (!marker) return;
    marker
      .fetch(block * BLOCK, BLOCK)
      .then((marks) => {
        if (generation !== this.generation || marker !== this.marker) return;
        this.marks.set(block, marks);
        this.invalidate();
      })
      .catch(() => {});
  }

  private fetchRowMarks(block: number, generation: number): void {
    const fetch = this.rowMarker;
    if (!fetch) return;
    fetch(block * BLOCK, BLOCK)
      .then((marks) => {
        if (generation !== this.generation || fetch !== this.rowMarker) return;
        this.rowMarks.set(block, marks);
        this.invalidate();
      })
      .catch(() => {});
  }

  private mark(row: number): number {
    return this.marks.get(Math.floor(row / BLOCK))?.[row % BLOCK] ?? 0;
  }

  private rowMark(row: number): number {
    return this.rowMarks.get(Math.floor(row / BLOCK))?.[row % BLOCK] ?? 0;
  }

  // Selection

  get selection(): Selection | null {
    return this.selected;
  }

  /** Selects cells, and with `reveal` scrolls the cursor into view. */
  select(selection: Selection | null, reveal = true): void {
    const rows = this.source?.rows ?? 0;
    const clamped = selection?.clamped(rows, this.widths.length) ?? null;
    const changed = !(clamped?.equals(this.selected) ?? this.selected === null);
    this.selected = clamped;
    if (clamped && reveal) this.reveal(clamped.cursor);
    this.invalidate();
    if (changed) this.onSelect?.(clamped);
  }

  /** Scrolls so that the cell is in view; frozen rows and columns always are. */
  reveal(cell: Cell): void {
    const height = this.rowHeight;
    let top = this.top;
    if (cell.row >= this.frozenRowCount) {
      const y = (cell.row - this.frozenRowCount) * height;
      const view = this.scrollHeight;
      if (y < top) top = y;
      else if (y + height > top + view) top = y + height - view;
    }
    let left = this.left;
    if (cell.column >= this.frozenColumnCount) {
      const x = this.edges[cell.column] - this.frozenWidth;
      const right = this.edges[cell.column + 1] - this.frozenWidth;
      const view = this.scrollWidth;
      if (x < left) left = x;
      else if (right > left + view) left = Math.min(x, right - view);
    }
    if (top !== this.top || left !== this.left) this.scrollTo(top, left);
  }

  /** Where the cell is on screen, relative to the grid, for an editor over it. */
  cellRect(cell: Cell): { x: number; y: number; width: number; height: number } {
    return { x: this.columnX(cell.column), y: this.rowY(cell.row), width: this.columnWidth(cell.column), height: this.rowHeight };
  }

  /** The value of a cell, if its rows are loaded. */
  value(cell: Cell): string | undefined {
    const row = this.row(cell.row);
    return row ? (row[cell.column] ?? "") : undefined;
  }

  focus(): void {
    this.keys.focus({ preventScroll: true });
  }

  /** Whether a key went to the grid itself, rather than to an editor or a field. */
  owns(target: EventTarget | null): boolean {
    return target === this.keys;
  }

  /**
   * Puts a selected space in the field: engines copy and cut only a selection, and typing
   * replaces it with what was typed.
   */
  private arm(): void {
    this.keys.value = " ";
    this.keys.select();
  }

  private typed(): void {
    const text = this.keys.value;
    this.arm();
    if (text) this.onType?.(text);
  }

  private clipboard(event: ClipboardEvent, cut: boolean): void {
    event.preventDefault();
    this.onCopy?.(cut);
  }

  /** Keeps the field for keys at the active cell, where input methods show their candidates. */
  private placeKeys(): void {
    const cursor = this.selected?.cursor;
    const rect = cursor ? this.cellRect(cursor) : { x: 0, y: 0, height: this.rowHeight };
    this.keys.style.left = `${Math.max(0, Math.min(this.width - 2, rect.x))}px`;
    this.keys.style.top = `${Math.max(0, Math.min(this.height - 20, rect.y))}px`;
  }

  // Geometry

  get rowHeight(): number {
    return LINE_ROW + (this.lines - 1) * MORE_LINE;
  }

  private get frozenWidth(): number {
    return this.edges[this.frozenColumnCount] ?? 0;
  }

  private get frozenHeight(): number {
    return this.frozenRowCount * this.rowHeight;
  }

  /** The height and width of the scrolling part. */
  private get scrollHeight(): number {
    return Math.max(0, this.height - HEADER_HEIGHT - this.frozenHeight);
  }

  private get scrollWidth(): number {
    return Math.max(0, this.width - this.gutter - this.frozenWidth);
  }

  private columnWidth(column: number): number {
    return this.hidden.has(column) ? 0 : (this.widths[column] ?? DEFAULT_WIDTH);
  }

  private columnX(column: number): number {
    const x = this.gutter + this.edges[column];
    return column < this.frozenColumnCount ? x : x - this.left;
  }

  private rowY(row: number): number {
    if (row < this.frozenRowCount) return HEADER_HEIGHT + row * this.rowHeight;
    return HEADER_HEIGHT + this.frozenHeight + (row - this.frozenRowCount) * this.rowHeight - this.top;
  }

  get maxTop(): number {
    const rows = this.source?.rows ?? 0;
    return Math.max(0, (rows - this.frozenRowCount) * this.rowHeight - this.scrollHeight);
  }

  get maxLeft(): number {
    return Math.max(0, this.edges[this.edges.length - 1] - this.frozenWidth - this.scrollWidth);
  }

  get scrollTop(): number {
    return this.top;
  }

  scrollTo(top: number, left = this.left): void {
    this.top = Math.min(Math.max(0, top), this.maxTop);
    this.left = Math.min(Math.max(0, left), this.maxLeft);
    this.place();
    this.invalidate();
  }

  /** The area the data rows take, relative to the grid: editors stay inside it. */
  get dataArea(): { x: number; y: number; width: number; height: number } {
    return { x: this.gutter, y: HEADER_HEIGHT, width: this.width - this.gutter, height: this.height - HEADER_HEIGHT };
  }

  /** The rows in view: frozen ones, then the scrolling ones, with where each is drawn. */
  private rowsInView(): { row: number; y: number }[] {
    const rows = this.source?.rows ?? 0;
    const height = this.rowHeight;
    const result: { row: number; y: number }[] = [];
    for (let row = 0; row < Math.min(this.frozenRowCount, rows); row += 1) result.push({ row, y: this.rowY(row) });
    const first = this.frozenRowCount + Math.floor(this.top / height);
    const end = Math.min(rows, first + Math.ceil(this.scrollHeight / height) + 1);
    for (let row = first; row < end; row += 1) result.push({ row, y: this.rowY(row) });
    return result;
  }

  /** The shown columns in view: frozen ones, then the scrolling ones. */
  private columnsInView(): number[] {
    const result: number[] = [];
    for (let column = 0; column < this.frozenColumnCount; column += 1) if (!this.hidden.has(column)) result.push(column);
    const start = this.frozenWidth + this.left;
    const end = start + this.scrollWidth;
    for (let column = this.frozenColumnCount; column < this.widths.length; column += 1) {
      if (this.hidden.has(column) || this.edges[column + 1] <= start) continue;
      if (this.edges[column] >= end) break;
      result.push(column);
    }
    return result;
  }

  /** Draws now instead of on the next frame; returns whether every row in view had its data. */
  flush(): boolean {
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
    return this.draw();
  }

  private invalidate(): void {
    if (!this.frame) {
      this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        this.draw();
      });
    }
  }

  private resize(): void {
    const dpr = window.devicePixelRatio || 1;
    const width = this.scroller.clientWidth;
    const height = this.scroller.clientHeight;
    if (width === this.width && height === this.height && dpr === this.dpr) return;
    this.dpr = dpr;
    this.width = width;
    this.height = height;
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
    this.canvas.width = Math.round(width * dpr);
    this.canvas.height = Math.round(height * dpr);
    this.layout();
    this.flush();
  }

  private layout(): void {
    const rows = this.source?.rows ?? 0;
    this.context.font = NUMBER_FONT;
    this.gutter = Math.ceil(this.context.measureText(String(Math.max(rows, 100))).width) + 2 * PADDING + 4;
    this.edges = [0];
    for (let column = 0; column < this.widths.length; column += 1) this.edges.push(this.edges[column] + this.columnWidth(column));
    this.frozenColumnCount = Math.min(this.frozenColumnCount, Math.max(0, this.widths.length - 1));
    this.frozenRowCount = Math.min(this.frozenRowCount, Math.max(0, rows - 1));

    const rangeY = Math.min(this.maxTop, MAX_SCROLL_RANGE);
    const rangeX = Math.min(this.maxLeft, MAX_SCROLL_RANGE);
    this.ratioY = this.maxTop > 0 ? rangeY / this.maxTop : 1;
    this.ratioX = this.maxLeft > 0 ? rangeX / this.maxLeft : 1;
    this.spacer.style.height = `${this.height + rangeY}px`;
    this.spacer.style.width = `${this.width + rangeX}px`;
    this.top = Math.min(this.top, this.maxTop);
    this.left = Math.min(this.left, this.maxLeft);
    this.place();
  }

  /** Moves the scrollbars to the table's position. */
  private place(): void {
    this.placed = { top: this.top * this.ratioY, left: this.left * this.ratioX };
    this.scroller.scrollTop = this.placed.top;
    this.scroller.scrollLeft = this.placed.left;
  }

  /** The scroller moved: by us, or by the scrollbar or keys the browser handles itself. */
  private scrolled(): void {
    const { scrollTop, scrollLeft } = this.scroller;
    // Browsers round scroll positions to device pixels.
    if (Math.abs(scrollTop - this.placed.top) > 1) this.top = Math.min(scrollTop / this.ratioY, this.maxTop);
    if (Math.abs(scrollLeft - this.placed.left) > 1) this.left = Math.min(scrollLeft / this.ratioX, this.maxLeft);
    this.placed = { top: scrollTop, left: scrollLeft };
    this.invalidate();
  }

  private wheel(event: WheelEvent): void {
    if (event.ctrlKey) return; // pinch to zoom
    event.preventDefault();
    const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? this.rowHeight : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? this.scrollHeight : 1;
    let dx = event.deltaX * unit;
    let dy = event.deltaY * unit;
    // Windows and Linux scroll sideways with Shift and the wheel.
    if (event.shiftKey && dx === 0) [dx, dy] = [dy, 0];
    this.scrollTo(this.top + dy, this.left + dx);
  }

  // Keys

  /** Rows a page moves: those in view, less one. */
  private get pageRows(): number {
    return Math.max(1, Math.floor(this.scrollHeight / this.rowHeight) - 1);
  }

  /** The next shown column from `column` in a direction, or `column` at the edge. */
  private step(column: number, direction: 1 | -1): number {
    for (let next = column + direction; next >= 0 && next < this.widths.length; next += direction) if (!this.hidden.has(next)) return next;
    return column;
  }

  /** Moves the cursor as the macOS app does (GridViewController.keyDown); other keys go on. */
  private key(event: KeyboardEvent): void {
    // Cmd on a Mac, Ctrl elsewhere, jumps to the edge.
    const jump = mac ? event.metaKey : event.ctrlKey;
    const other = mac ? event.ctrlKey : event.metaKey;
    if (other || event.altKey) return;
    const rows = this.source?.rows ?? 0;
    const columns = this.widths.length;
    if (jump && !event.shiftKey && event.key.toLowerCase() === "a") {
      event.preventDefault();
      if (rows > 0 && columns > 0) this.select(new Selection({ row: rows - 1, column: columns - 1 }, { row: 0, column: 0 }), false);
      return;
    }
    if (event.key === "Escape" && this.selected && !this.selected.single) {
      event.preventDefault();
      this.select(Selection.at(this.selected.cursor), false);
      return;
    }
    if (rows === 0 || columns === 0) return;
    const at = this.selected?.cursor ?? { row: 0, column: this.shownColumn(0) ?? 0 };
    const lastRow = rows - 1;
    let to: Cell | null = null;
    let extend = event.shiftKey;
    switch (event.key) {
      case "ArrowDown":
        to = { row: jump ? lastRow : at.row + 1, column: at.column };
        break;
      case "ArrowUp":
        to = { row: jump ? 0 : at.row - 1, column: at.column };
        break;
      case "ArrowRight":
        to = { row: at.row, column: jump ? (this.shownColumn(columns - 1, -1) ?? at.column) : this.step(at.column, 1) };
        break;
      case "ArrowLeft":
        to = { row: at.row, column: jump ? (this.shownColumn(0) ?? at.column) : this.step(at.column, -1) };
        break;
      case "PageDown":
        to = { row: at.row + this.pageRows, column: at.column };
        break;
      case "PageUp":
        to = { row: at.row - this.pageRows, column: at.column };
        break;
      case "Home":
        to = { row: 0, column: at.column };
        break;
      case "End":
        to = { row: lastRow, column: at.column };
        break;
      case "Tab":
        to = { row: at.row, column: this.step(at.column, event.shiftKey ? -1 : 1) };
        extend = false;
        break;
      default:
        return;
    }
    event.preventDefault();
    const cell = { row: Math.max(0, Math.min(lastRow, to.row)), column: Math.max(0, Math.min(columns - 1, to.column)) };
    this.select(this.selected ? this.selected.moved(cell, extend) : Selection.at(cell));
  }

  // The pointer

  private point(clientX: number, clientY: number): { x: number; y: number } {
    const bounds = this.scroller.getBoundingClientRect();
    return { x: clientX - bounds.left, y: clientY - bounds.top };
  }

  /** The row at `y` of the canvas, frozen or scrolling; may be past the last. */
  private rowAt(y: number): number {
    const height = this.rowHeight;
    if (y < HEADER_HEIGHT + this.frozenHeight) return Math.floor((y - HEADER_HEIGHT) / height);
    return this.frozenRowCount + Math.floor((y - HEADER_HEIGHT - this.frozenHeight + this.top) / height);
  }

  /** The shown column at `x` of the canvas, frozen or scrolling, or `null` past the last. */
  private columnAt(x: number): number | null {
    const inTable = x - this.gutter < this.frozenWidth ? x - this.gutter : x - this.gutter + this.left;
    if (inTable < 0 || inTable >= this.edges[this.edges.length - 1]) return null;
    let low = 0;
    let high = this.widths.length - 1;
    while (low < high) {
      const middle = (low + high + 1) >> 1;
      if (this.edges[middle] <= inTable) low = middle;
      else high = middle - 1;
    }
    // Hidden columns take no room: the shown one at this edge.
    while (low < this.widths.length - 1 && this.hidden.has(low)) low += 1;
    return low;
  }

  private hit(clientX: number, clientY: number, clamp = false): Hit {
    let { x, y } = this.point(clientX, clientY);
    if (clamp) {
      x = Math.max(this.gutter, Math.min(this.width - 1, x));
      y = Math.max(HEADER_HEIGHT, Math.min(this.height - 1, y));
    }
    if (x < 0 || y < 0 || x >= this.width || y >= this.height) return { area: "none" };
    const rows = this.source?.rows ?? 0;
    const column = this.columnAt(x);
    if (y < HEADER_HEIGHT) return x < this.gutter ? { area: "corner" } : column === null ? { area: "none" } : { area: "header", column };
    const row = Math.min(rows - 1, this.rowAt(y));
    if (row < 0) return { area: "none" };
    if (x < this.gutter) return { area: "gutter", row };
    return column === null ? { area: "none" } : { area: "cell", row, column };
  }

  /** The column whose right edge is under `x` in the titles, to resize. */
  private edgeAt(x: number): number | null {
    for (const column of this.columnsInView()) {
      const right = this.columnX(column) + this.columnWidth(column);
      if (Math.abs(x - right) <= RESIZE_GRIP) return column;
    }
    return null;
  }

  private pointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    const { x, y } = this.point(event.clientX, event.clientY);
    const hit = this.hit(event.clientX, event.clientY);
    if (hit.area === "none" && y >= HEADER_HEIGHT) return;
    event.preventDefault();
    this.focus();
    if (y < HEADER_HEIGHT) {
      const edge = this.edgeAt(x);
      if (edge !== null) {
        this.capture(event, { kind: "resize", column: edge, x, width: this.widths[edge] });
        return;
      }
      if (hit.area !== "header") return;
      const icon = this.filterIconX(hit.column);
      // The icon's area reaches a little past it, as in the macOS app.
      if (this.showsFilterIcon(hit.column) && x >= icon - 3 && x <= icon + ICON + 3) {
        this.onFilterIcon?.(hit.column);
        return;
      }
      // A click sorts when the pointer comes up; a drag moves the column.
      this.capture(event, { kind: "title", column: hit.column, x, y, shift: event.shiftKey, moving: false, target: hit.column });
      return;
    }
    const columns = this.widths.length;
    if (hit.area === "gutter") {
      const range = this.selected?.range;
      const wholeRows = !!range && range.left === 0 && range.right === columns - 1;
      // Pressing a selected whole row's number when rows can move starts dragging them.
      if (!event.shiftKey && wholeRows && hit.row >= range.top && hit.row <= range.bottom && this.canMoveRows()) {
        this.capture(event, { kind: "rows", x, y, moving: false, target: hit.row, frame: 0 });
        return;
      }
      // A row: from its last column to its first, so the cursor is at its start.
      const anchor = event.shiftKey && this.selected ? { row: this.selected.anchor.row, column: columns - 1 } : { row: hit.row, column: columns - 1 };
      this.select(new Selection(anchor, { row: hit.row, column: 0 }), false);
    } else if (hit.area === "cell") {
      const cell = { row: hit.row, column: hit.column };
      this.select(event.shiftKey && this.selected ? this.selected.moved(cell, true) : Selection.at(cell), false);
    }
    this.capture(event, { kind: "select", rows: hit.area === "gutter", x: event.clientX, y: event.clientY, frame: 0 });
  }

  private capture(event: PointerEvent, press: Press): void {
    this.press = press;
    this.scroller.setPointerCapture(event.pointerId);
  }

  private pointerMove(event: PointerEvent): void {
    const press = this.press;
    const { x, y } = this.point(event.clientX, event.clientY);
    if (!press) {
      this.hover(event, x, y);
      return;
    }
    if (press.kind === "resize") {
      this.widths[press.column] = Math.max(MIN_WIDTH, Math.round(press.width + x - press.x));
      this.layout();
      this.invalidate();
    } else if (press.kind === "title") {
      if (!press.moving && Math.abs(x - press.x) < DRAG_START) return;
      press.moving = true;
      press.target = this.columnSlot(x);
      this.invalidate();
    } else if (press.kind === "rows") {
      if (!press.moving && Math.abs(y - press.y) < DRAG_START) return;
      press.moving = true;
      press.x = event.clientX;
      press.y = event.clientY;
      press.target = this.rowSlot(y);
      this.invalidate();
      if (!press.frame) press.frame = requestAnimationFrame(() => this.autoscroll());
    } else {
      press.x = event.clientX;
      press.y = event.clientY;
      this.extendDrag();
      if (!press.frame) press.frame = requestAnimationFrame(() => this.autoscroll());
    }
  }

  /** Icons of the title under the pointer, a resize cursor at an edge, and tooltips of cells. */
  private hover(event: PointerEvent, x: number, y: number): void {
    const hit = this.hit(event.clientX, event.clientY);
    let tip = "";
    if (hit.area === "cell") {
      const mark = this.marker && hit.column === this.marker.column ? this.mark(hit.row) : 0;
      const row = this.row(hit.row);
      if (mark) tip = this.marker!.describe(mark);
      else if (this.invisibles && row && hit.column >= row.length) tip = this.missingText;
    }
    if (this.scroller.title !== tip) this.scroller.title = tip;
    const resizing = y < HEADER_HEIGHT && this.edgeAt(x) !== null;
    this.scroller.style.cursor = resizing ? "col-resize" : "";
    const hovered = hit.area === "header" ? hit.column : null;
    if (hovered !== this.hovered) {
      this.hovered = hovered;
      this.invalidate();
    }
  }

  /** Where a dragged column would go: before the column under the pointer, or after it. */
  private columnSlot(x: number): number {
    const column = this.columnAt(x);
    if (column === null) return x < this.gutter ? 0 : this.widths.length;
    const middle = this.columnX(column) + this.columnWidth(column) / 2;
    return x > middle ? column + 1 : column;
  }

  /** Where dragged rows would go: before the row under the pointer, or after it. */
  private rowSlot(y: number): number {
    const rows = this.source?.rows ?? 0;
    if (y < HEADER_HEIGHT) return 0;
    const row = this.rowAt(y);
    if (row >= rows) return rows;
    const middle = this.rowY(row) + this.rowHeight / 2;
    return y > middle ? row + 1 : row;
  }

  private pointerUp(event: PointerEvent): void {
    const press = this.press;
    if (!press) return;
    this.press = null;
    if (this.scroller.hasPointerCapture(event.pointerId)) this.scroller.releasePointerCapture(event.pointerId);
    if (press.kind === "select" || press.kind === "rows") cancelAnimationFrame(press.frame);
    if (press.kind === "title") {
      if (!press.moving) this.onHeader?.(press.column, press.shift);
      else {
        // The slot counts columns before the move; the core takes the place after it.
        const to = press.target > press.column ? press.target - 1 : press.target;
        if (to !== press.column) this.onMoveColumn?.(press.column, to);
      }
      this.invalidate();
    } else if (press.kind === "rows") {
      const range = this.selected?.range;
      if (press.moving && range && (press.target < range.top || press.target > range.bottom + 1)) this.onMoveRows?.(press.target);
      else {
        const row = Math.min(Math.max(press.target, 0), (this.source?.rows ?? 1) - 1);
        this.select(new Selection({ row, column: this.widths.length - 1 }, { row, column: 0 }), false);
      }
      this.invalidate();
    }
  }

  /** Extends the selection to the cell under the pointer while dragging. */
  private extendDrag(): void {
    const press = this.press;
    if (!press || press.kind !== "select" || !this.selected) return;
    const hit = this.hit(press.x, press.y, true);
    if (hit.area !== "cell") return;
    const cell = press.rows ? { row: hit.row, column: 0 } : { row: hit.row, column: hit.column };
    const selection = this.selected.moved(cell, true);
    if (!selection.equals(this.selected)) this.select(selection, false);
  }

  /** Scrolls while the pointer drags past an edge. */
  private autoscroll(): void {
    const press = this.press;
    if (!press || (press.kind !== "select" && press.kind !== "rows")) return;
    press.frame = 0;
    const bounds = this.scroller.getBoundingClientRect();
    const speed = (overshoot: number) => Math.sign(overshoot) * Math.min(AUTOSCROLL_STEP, Math.abs(overshoot));
    const below = press.y - (bounds.top + this.height - AUTOSCROLL_EDGE);
    const above = bounds.top + HEADER_HEIGHT + this.frozenHeight + AUTOSCROLL_EDGE - press.y;
    const right = press.x - (bounds.left + this.width - AUTOSCROLL_EDGE);
    const left = bounds.left + this.gutter + this.frozenWidth + AUTOSCROLL_EDGE - press.x;
    const sideways = press.kind === "select" && !press.rows;
    const dy = below > 0 ? speed(below) : above > 0 ? -speed(above) : 0;
    const dx = !sideways ? 0 : right > 0 ? speed(right) : left > 0 ? -speed(left) : 0;
    if (dx === 0 && dy === 0) return;
    this.scrollTo(this.top + dy, this.left + dx);
    if (press.kind === "select") this.extendDrag();
    else press.target = this.rowSlot(press.y - bounds.top);
    press.frame = requestAnimationFrame(() => this.autoscroll());
  }

  /**
   * A cell outside the selection becomes the selection; a column's title moves the cursor to
   * that column in the active row, as the macOS app's menus do.
   */
  private contextMenu(event: MouseEvent): void {
    const hit = this.hit(event.clientX, event.clientY);
    if (hit.area !== "cell" && hit.area !== "header") return;
    event.preventDefault();
    this.focus();
    if (hit.area === "cell") {
      const cell = { row: hit.row, column: hit.column };
      if (!this.selected?.contains(cell.row, cell.column)) this.select(Selection.at(cell), false);
      this.onMenu?.({ area: "cell", cell }, event.clientX, event.clientY);
    } else {
      const row = this.selected?.cursor.row ?? 0;
      if ((this.source?.rows ?? 0) > 0) this.select(Selection.at({ row, column: hit.column }), false);
      this.onMenu?.({ area: "header", column: hit.column }, event.clientX, event.clientY);
    }
  }

  /** A double click edits a cell, and fits a column at its title's edge. */
  private doubleClick(event: MouseEvent): void {
    const { x, y } = this.point(event.clientX, event.clientY);
    if (y < HEADER_HEIGHT) {
      const edge = this.edgeAt(x);
      if (edge !== null) void this.fitColumns([edge]);
      return;
    }
    const hit = this.hit(event.clientX, event.clientY);
    if (hit.area === "cell") this.onActivate?.({ row: hit.row, column: hit.column });
  }

  // Rows

  private row(index: number): string[] | undefined {
    return this.blocks.get(Math.floor(index / BLOCK))?.[index % BLOCK];
  }

  private number(index: number): number | undefined {
    return this.numbers.get(Math.floor(index / BLOCK))?.[index % BLOCK];
  }

  /** Asks for the blocks of the rows in view, and one on each side, dropping requests no longer in view. */
  private request(rows: number[]): void {
    const total = this.source?.rows ?? 0;
    if (rows.length === 0 || total === 0) return;
    const inView = new Set(rows.map((row) => Math.floor(row / BLOCK)));
    const near = new Set<number>();
    for (const block of inView) {
      near.add(block - 1);
      near.add(block + 1);
    }
    const wanted = [...inView, ...[...near].filter((block) => !inView.has(block))].filter(
      (block) => block >= 0 && block * BLOCK < total && !this.blocks.has(block) && !this.pending.has(block),
    );
    this.queue = wanted;
    this.pump();
  }

  private pump(): void {
    const source = this.source;
    if (!source) return;
    while (this.inFlight < MAX_IN_FLIGHT && this.queue.length > 0) {
      const block = this.queue.shift()!;
      const generation = this.generation;
      const started = performance.now();
      this.pending.add(block);
      this.inFlight += 1;
      source
        .fetch(block * BLOCK, BLOCK)
        .then(({ rows, numbers }) => {
          if (generation !== this.generation) return;
          this.stats.fetch(performance.now() - started);
          this.blocks.set(block, rows);
          this.numbers.set(block, numbers);
          this.fetchMarks(block, generation);
          this.fetchRowMarks(block, generation);
          this.evict(block);
          this.invalidate();
        })
        .catch((error: unknown) => console.error("rows", block * BLOCK, error))
        .finally(() => {
          if (generation !== this.generation) return;
          this.pending.delete(block);
          this.inFlight -= 1;
          this.pump();
        });
    }
  }

  /** Keeps the blocks nearest to the view. */
  private evict(near: number): void {
    if (this.blocks.size <= MAX_BLOCKS) return;
    const far = [...this.blocks.keys()].sort((a, b) => Math.abs(b - near) - Math.abs(a - near));
    for (const block of far.slice(0, this.blocks.size - MAX_BLOCKS)) {
      this.blocks.delete(block);
      this.numbers.delete(block);
      this.marks.delete(block);
      this.rowMarks.delete(block);
    }
  }

  private isFiltered(column: number): boolean {
    return this.filtered.has(column);
  }

  private showsFilterIcon(column: number): boolean {
    return this.hovered === column || this.isFiltered(column);
  }

  /** The left of a column's filter icon. */
  private filterIconX(column: number): number {
    const right = this.columnX(column) + this.columnWidth(column);
    return right - SORT_ICON_INSET - ICON - FILTER_ICON_GAP - ICON;
  }

  /** The header's title of a column, or its letter as in Excel. */
  title(column: number): string {
    const title = this.source?.header[column];
    if (title) return title.replace(/\r\n|\r|\n/g, " ");
    let name = "";
    for (let n = column + 1; n > 0; n = Math.floor((n - 1) / 26)) name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
    return name;
  }

  // Drawing

  private readColors(): Colors {
    const style = getComputedStyle(this.host);
    const color = (name: string) => style.getPropertyValue(name).trim();
    return {
      background: color("--grid-background"),
      alternate: color("--grid-alternate"),
      text: color("--grid-text"),
      muted: color("--grid-muted"),
      quiet: color("--grid-quiet"),
      line: color("--grid-line"),
      header: color("--grid-header"),
      headerText: color("--grid-header-text"),
      placeholder: color("--grid-placeholder"),
      selection: color("--grid-selection"),
      cursor: color("--accent"),
      selectedHeader: color("--grid-selected-header"),
      issue: color("--grid-issue"),
      mark: color("--grid-mark"),
      markBackground: color("--grid-mark-background"),
      missing: color("--grid-missing"),
    };
  }

  /** chevron.up or chevron.down when sorted, arrow.up.arrow.down when only hovered. */
  private drawSortIcon(x: number, sort: { ascending: boolean } | undefined, color: string): void {
    const g = this.context;
    const middle = HEADER_HEIGHT / 2;
    g.save();
    g.strokeStyle = color;
    g.lineWidth = 1.6;
    g.lineCap = "round";
    g.lineJoin = "round";
    g.beginPath();
    if (sort) {
      const direction = sort.ascending ? -1 : 1;
      g.moveTo(x + 4, middle - direction * 2.5);
      g.lineTo(x + 8, middle + direction * 2.5);
      g.lineTo(x + 12, middle - direction * 2.5);
    } else {
      g.lineWidth = 1.2;
      g.moveTo(x + 5.5, middle + 4.5);
      g.lineTo(x + 5.5, middle - 4.5);
      g.moveTo(x + 3.5, middle - 2.5);
      g.lineTo(x + 5.5, middle - 4.5);
      g.lineTo(x + 7.5, middle - 2.5);
      g.moveTo(x + 10.5, middle - 4.5);
      g.lineTo(x + 10.5, middle + 4.5);
      g.moveTo(x + 8.5, middle + 2.5);
      g.lineTo(x + 10.5, middle + 4.5);
      g.lineTo(x + 12.5, middle + 2.5);
    }
    g.stroke();
    g.restore();
  }

  /** line.3.horizontal.decrease.circle. */
  private drawFilterIcon(x: number, color: string): void {
    const g = this.context;
    const middle = HEADER_HEIGHT / 2;
    g.save();
    g.strokeStyle = color;
    g.lineWidth = 1.2;
    g.lineCap = "round";
    g.beginPath();
    g.arc(x + 8, middle, 6.5, 0, Math.PI * 2);
    g.moveTo(x + 4.8, middle - 2.2);
    g.lineTo(x + 11.2, middle - 2.2);
    g.moveTo(x + 5.9, middle);
    g.lineTo(x + 10.1, middle);
    g.moveTo(x + 7, middle + 2.2);
    g.lineTo(x + 9, middle + 2.2);
    g.stroke();
    g.restore();
  }

  /** Draws a cell's text: on one line, clipped, or wrapped; with invisible characters or not. */
  private drawText(value: string, x: number, y: number, width: number): void {
    const g = this.context;
    const height = this.rowHeight;
    const room = width - 2 * PADDING;
    if (room <= 0) return;
    // The common case stays cheap: plain text on one line, cut off by the clip.
    if (!this.invisibles && this.lines === 1) {
      const characters = Math.ceil(width / 4) + 2;
      let text = value.length > characters ? value.slice(0, characters) : value;
      if (text.includes("\n") || text.includes("\r")) text = text.replace(/\r\n|\r|\n/g, " ↵ ");
      g.fillStyle = this.colors.text;
      g.fillText(text, x + PADDING, y + height / 2);
      return;
    }
    const wraps = this.lines > 1;
    const limited = value.length > 2000 ? value.slice(0, 2000) : value;
    const runs: Run[] = this.invisibles ? invisibleRuns(limited, wraps) : plainRuns(limited, wraps);
    const lines = wraps ? wrap(runs, room, this.lines, (text) => g.measureText(text).width) : [runs];
    // Lines sit in the middle of the row, as the macOS app centres its text box.
    const first = y + height / 2 - ((lines.length - 1) * MORE_LINE) / 2;
    lines.forEach((line, index) => {
      let left = x + PADDING;
      const middle = first + index * MORE_LINE;
      for (const run of line) {
        const runWidth = g.measureText(run.text).width;
        if (run.style === "mark") {
          g.fillStyle = this.colors.markBackground;
          g.fillRect(left, middle - 8, runWidth, 16);
        }
        g.fillStyle = run.style === "mark" ? this.colors.mark : run.style === "quiet" ? this.colors.quiet : this.colors.text;
        g.fillText(run.text, left, middle);
        left += runWidth;
        if (left > x + width) break;
      }
    });
  }

  private draw(): boolean {
    if ((window.devicePixelRatio || 1) !== this.dpr) this.resize();
    const started = performance.now();
    const { context: g, colors, width, height, gutter, dpr } = this;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = colors.background;
    g.fillRect(0, 0, width, height);
    const source = this.source;
    if (!source) return true;

    const rowHeight = this.rowHeight;
    const rows = this.rowsInView();
    const columns = this.columnsInView();
    this.request(rows.map(({ row }) => row));
    // Lines fall on device pixels.
    const crisp = (value: number) => (Math.round(value * dpr) + 0.5) / dpr;
    const frozenBottom = HEADER_HEIGHT + this.frozenHeight;
    const frozenRight = gutter + this.frozenWidth;
    /** The part of the canvas of frozen or scrolling rows and columns. */
    const part = (frozenRows: boolean, frozenColumns: boolean): [number, number, number, number] => {
      const top = frozenRows ? HEADER_HEIGHT : frozenBottom;
      const bottom = frozenRows ? frozenBottom : height;
      const left = frozenColumns ? gutter : frozenRight;
      const right = frozenColumns ? frozenRight : width;
      return [left, top, right - left, bottom - top];
    };
    const clipFor = (row: number, column: number) => part(row < this.frozenRowCount, column < this.frozenColumnCount);
    const selection = this.selected?.range;
    const marked = this.marker?.column;
    let complete = true;

    // Each part of the view: its rows' backgrounds, then its cells.
    for (const frozenRows of [false, true]) {
      for (const frozenColumns of [false, true]) {
        const partRows = rows.filter(({ row }) => row < this.frozenRowCount === frozenRows);
        const partColumns = columns.filter((column) => column < this.frozenColumnCount === frozenColumns);
        if (partRows.length === 0 || partColumns.length === 0) continue;
        const [left, top, clipWidth, clipHeight] = part(frozenRows, frozenColumns);
        if (clipWidth <= 0 || clipHeight <= 0) continue;
        g.save();
        g.beginPath();
        g.rect(left, top, clipWidth, clipHeight);
        g.clip();
        for (const { row, y } of partRows) {
          if (row % 2 === 1) {
            g.fillStyle = colors.alternate;
            g.fillRect(left, y, clipWidth, rowHeight);
          }
          const values = this.row(row);
          if (!values) complete = false;
          for (const column of partColumns) {
            const x = this.columnX(column);
            const cellWidth = this.columnWidth(column);
            if (!values) {
              g.fillStyle = colors.placeholder;
              g.fillRect(x + PADDING, y + rowHeight / 2 - 4, Math.min(cellWidth - 2 * PADDING, 48), 8);
              continue;
            }
            const selected = !!selection && row >= selection.top && row <= selection.bottom && column >= selection.left && column <= selection.right;
            if (selected) {
              g.fillStyle = colors.selection;
              g.fillRect(x, y, cellWidth, rowHeight);
            } else if (column === marked && this.mark(row)) {
              g.fillStyle = colors.issue;
              g.fillRect(x, y, cellWidth, rowHeight);
            } else if (this.invisibles && column >= values.length) {
              // A field the row does not have.
              g.fillStyle = colors.missing;
              g.fillRect(x, y, cellWidth, rowHeight);
            }
            if (column >= values.length) continue;
            g.save();
            g.beginPath();
            g.rect(x, y, cellWidth - PADDING / 2, rowHeight);
            g.clip();
            g.font = FONT;
            g.textBaseline = "middle";
            g.textAlign = "left";
            this.drawText(values[column], x, y, cellWidth);
            g.restore();
          }
        }
        // Grid lines between the columns and under the rows.
        g.strokeStyle = colors.line;
        g.lineWidth = 1 / dpr;
        g.beginPath();
        for (const column of partColumns) {
          const x = crisp(this.columnX(column) + this.columnWidth(column));
          g.moveTo(x, top);
          g.lineTo(x, top + clipHeight);
        }
        if (this.lines > 1) {
          for (const { y } of partRows) {
            g.moveTo(left, crisp(y + rowHeight));
            g.lineTo(left + clipWidth, crisp(y + rowHeight));
          }
        }
        g.stroke();
        g.restore();
      }
    }

    // The active cell, in its part of the view.
    const cursor = this.selected?.cursor;
    if (cursor && !this.hidden.has(cursor.column) && rows.some(({ row }) => row === cursor.row)) {
      const [left, top, clipWidth, clipHeight] = clipFor(cursor.row, cursor.column);
      g.save();
      g.beginPath();
      g.rect(left, top, clipWidth, clipHeight);
      g.clip();
      g.strokeStyle = colors.cursor;
      g.lineWidth = 2;
      g.strokeRect(this.columnX(cursor.column) + 1, this.rowY(cursor.row) + 1, this.columnWidth(cursor.column) - 2, rowHeight - 2);
      g.restore();
    }

    // Lines under the frozen rows and beside the frozen columns.
    g.strokeStyle = colors.line;
    g.lineWidth = 1 / dpr;
    g.beginPath();
    if (this.frozenRowCount > 0) {
      g.moveTo(0, crisp(frozenBottom));
      g.lineTo(width, crisp(frozenBottom));
    }
    if (this.frozenColumnCount > 0) {
      g.moveTo(crisp(frozenRight), 0);
      g.lineTo(crisp(frozenRight), height);
    }
    g.stroke();

    // Row numbers: their place in the file, as the macOS app numbers them.
    g.fillStyle = colors.header;
    g.fillRect(0, HEADER_HEIGHT, gutter, height - HEADER_HEIGHT);
    g.font = NUMBER_FONT;
    g.textBaseline = "middle";
    g.textAlign = "right";
    for (const { row, y } of rows) {
      g.save();
      g.beginPath();
      g.rect(0, row < this.frozenRowCount ? HEADER_HEIGHT : frozenBottom, gutter, row < this.frozenRowCount ? this.frozenHeight : height - frozenBottom);
      g.clip();
      const selected = !!selection && row >= selection.top && row <= selection.bottom;
      if (this.rowMark(row)) {
        g.fillStyle = colors.issue;
        g.fillRect(0, y, gutter, rowHeight);
      } else if (selected) {
        g.fillStyle = colors.selectedHeader;
        g.fillRect(0, y, gutter, rowHeight);
      }
      g.fillStyle = selected ? colors.cursor : colors.muted;
      // With several lines, the number stands by the first.
      const middle = this.lines > 1 ? y + rowHeight / 2 - ((this.lines - 1) * MORE_LINE) / 2 : y + rowHeight / 2;
      g.fillText(String(this.number(row) ?? row + 1), gutter - PADDING, middle);
      g.restore();
    }

    // Where dragged rows would go.
    const press = this.press;
    if (press?.kind === "rows" && press.moving) {
      const y = press.target < (source.rows ?? 0) ? this.rowY(press.target) : this.rowY(source.rows - 1) + rowHeight;
      if (y >= HEADER_HEIGHT && y <= height) {
        g.fillStyle = colors.cursor;
        g.fillRect(0, y - 1, width, 2);
      }
    }

    // Column titles.
    g.fillStyle = colors.header;
    g.fillRect(0, 0, width, HEADER_HEIGHT);
    // The column of the first sort key stands out, as NSTableView's highlighted column.
    const highlighted = [...this.sorted].find(([, key]) => key.order === 0)?.[0];
    for (const column of columns) {
      const x = this.columnX(column);
      const cellWidth = this.columnWidth(column);
      const left = column < this.frozenColumnCount ? gutter : frozenRight;
      g.save();
      g.beginPath();
      g.rect(left, 0, (column < this.frozenColumnCount ? frozenRight : width) - left, HEADER_HEIGHT);
      g.clip();
      const inSelection = !!selection && column >= selection.left && column <= selection.right;
      if (inSelection || column === highlighted) {
        g.fillStyle = colors.selectedHeader;
        g.fillRect(x, 0, cellWidth, HEADER_HEIGHT);
      }
      // Icons at the right: sort when sorted or hovered, filter when filtered or hovered.
      const sort = this.sorted.get(column);
      const showsSort = !!sort || this.hovered === column;
      const showsFilter = this.showsFilterIcon(column);
      const iconsWidth = showsFilter ? SORT_ICON_INSET + 2 * ICON + FILTER_ICON_GAP : showsSort ? SORT_ICON_INSET + ICON : 0;
      g.save();
      g.beginPath();
      g.rect(x, 0, Math.max(0, cellWidth - PADDING / 2 - iconsWidth), HEADER_HEIGHT);
      g.clip();
      g.font = HEADER_FONT;
      g.textBaseline = "middle";
      g.textAlign = "left";
      g.fillStyle = colors.headerText;
      g.fillText(this.title(column), x + PADDING, HEADER_HEIGHT / 2);
      g.restore();
      if (showsSort) this.drawSortIcon(x + cellWidth - SORT_ICON_INSET - ICON, sort, sort ? colors.cursor : colors.muted);
      if (showsFilter) this.drawFilterIcon(this.filterIconX(column), this.isFiltered(column) ? colors.cursor : colors.muted);
      g.restore();
    }
    g.fillStyle = colors.header;
    g.fillRect(0, 0, gutter, HEADER_HEIGHT);

    g.strokeStyle = colors.line;
    g.lineWidth = 1 / dpr;
    g.beginPath();
    g.moveTo(0, crisp(HEADER_HEIGHT - 1));
    g.lineTo(width, crisp(HEADER_HEIGHT - 1));
    g.moveTo(crisp(gutter - 1), 0);
    g.lineTo(crisp(gutter - 1), height);
    for (const column of columns) {
      const x = crisp(this.columnX(column) + this.columnWidth(column));
      if (x < gutter || (column >= this.frozenColumnCount && x < frozenRight)) continue;
      g.moveTo(x, 0);
      g.lineTo(x, HEADER_HEIGHT);
    }
    g.stroke();

    // Where a dragged column would go.
    if (press?.kind === "title" && press.moving) {
      const x = press.target < this.widths.length ? this.columnX(press.target) : this.columnX(this.widths.length - 1) + this.columnWidth(this.widths.length - 1);
      g.fillStyle = colors.cursor;
      g.fillRect(x - 1, 0, 2, height);
    }

    this.stats.draw(performance.now() - started);
    this.placeKeys();
    for (const listener of this.afterDraw) listener();
    this.onFrame?.(complete);
    return complete;
  }
}
