// The table: a canvas that draws only the cells in view, over a scroller whose scrollbars stand
// for the whole table. Rows come from the core in blocks, as the view reaches them.
//
// A table of 60 million rows is 1.4 billion pixels tall, far past the largest element a
// browser lays out (about 33 million px in Chromium, less in WebKit). So the scroll position
// lives here, in pixels of the table, and the scroller's range is that position scaled down:
// wheel and keys move the table by their own pixels, the scrollbar thumb by its share.

import type { ViewSpec } from "../lib/api";
import { Selection, type Cell } from "./selection";
import { Stats } from "./stats";

/** The rows of a table, as the core shows them with the current sort and filters. */
export interface TableSource {
  /** Data rows, without the header. */
  readonly rows: number;
  readonly columns: number;
  /** Column titles; empty when the file has no header row. */
  readonly header: readonly string[];
  /** Up to `count` rows from `start`; fewer at the end. */
  fetch(start: number, count: number): Promise<string[][]>;
}

const ROW_HEIGHT = 24;
const HEADER_HEIGHT = 28;
const PADDING = 8;
const MIN_WIDTH = 56;
/** Room beside a column's title for its sort icon. */
const MARK_ROOM = 18;
/** The icons at the right of a column's title, as GridHeaderView draws them. */
const ICON = 16;
const SORT_ICON_INSET = 5;
const FILTER_ICON_GAP = 3;
const MAX_WIDTH = 360;
const DEFAULT_WIDTH = 120;
/** Columns measured for their width; the rest keep the default. */
const MEASURED_COLUMNS = 200;
/** Rows per request to the core. */
const BLOCK = 128;
/** Blocks kept, about 32 thousand rows. */
const MAX_BLOCKS = 256;
const MAX_IN_FLIGHT = 3;
/** Pixels the scrollbars stand for at most; beyond that the scroller's range is scaled. */
const MAX_SCROLL_RANGE = 4_000_000;
const FONT = '13px system-ui, -apple-system, "Segoe UI", Ubuntu, Cantarell, sans-serif';
const HEADER_FONT = `600 ${FONT}`;
/** Pixels from an edge where dragging a selection scrolls, and the most it scrolls a frame. */
const AUTOSCROLL_EDGE = 24;
const AUTOSCROLL_STEP = 40;
const mac = navigator.userAgent.includes("Mac");

interface Colors {
  background: string;
  alternate: string;
  text: string;
  muted: string;
  line: string;
  header: string;
  headerText: string;
  placeholder: string;
  selection: string;
  cursor: string;
  selectedHeader: string;
}

/** What is under a point of the grid. */
type Hit =
  | { area: "cell"; row: number; column: number }
  | { area: "header"; column: number }
  | { area: "gutter"; row: number }
  | { area: "corner" | "none" };

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
  private top = 0;
  private left = 0;
  /** Where the scroller was last put, to tell its own scrolling from ours. */
  private placed = { top: 0, left: 0 };
  private ratioY = 1;
  private ratioX = 1;

  private gutter = 48;
  private widths: number[] = [];
  /** Left edge of each column from the gutter, and the total width at the end. */
  private edges: number[] = [0];
  private measured = false;

  private selected: Selection | null = null;
  /** Sorted columns: ascending or not, and their place among several keys. */
  private sorted = new Map<number, { ascending: boolean; order: number }>();
  private filtered = new Set<number | null>();
  /** The column whose title the pointer is over: its icons show. */
  private hovered: number | null = null;
  /** A drag selecting cells or rows, and where the pointer last was. */
  private drag: { rows: boolean; x: number; y: number; frame: number } | null = null;

  private readonly blocks = new Map<number, string[][]>();
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

  /** Shows another table, from the top. */
  setSource(source: TableSource | null): void {
    this.source = source;
    this.generation += 1;
    this.blocks.clear();
    this.pending.clear();
    this.queue = [];
    this.inFlight = 0;
    this.top = 0;
    this.left = 0;
    this.measured = false;
    this.selected = null;
    this.widths = Array.from({ length: source?.columns ?? 0 }, (_, column) =>
      column < MEASURED_COLUMNS ? this.fit(MIN_WIDTH, HEADER_FONT, [this.title(column)]) + MARK_ROOM : DEFAULT_WIDTH,
    );
    this.layout();
    this.invalidate();
    this.focus();
  }

  /** The rows changed: an edit, undo, sort or filter. Keeps the place and the column widths. */
  reload(): void {
    this.generation += 1;
    this.blocks.clear();
    this.pending.clear();
    this.queue = [];
    this.inFlight = 0;
    this.update();
  }

  /** The table grew: more rows were indexed, or columns appeared. */
  update(): void {
    const columns = this.source?.columns ?? 0;
    while (this.widths.length < columns) this.widths.push(DEFAULT_WIDTH);
    // The last block may have been cut short by the end of what was indexed then.
    for (const [index, rows] of this.blocks) if (rows.length < BLOCK) this.blocks.delete(index);
    this.layout();
    const clamped = this.selected?.clamped(this.source?.rows ?? 0, columns) ?? null;
    if (!(clamped?.equals(this.selected) ?? this.selected === null)) this.select(clamped, false);
    this.invalidate();
  }

  /** Shows which columns sort the rows and which are filtered. */
  setView(view: ViewSpec): void {
    this.sorted = new Map(view.sort.map((key, order) => [key.column, { ascending: key.ascending, order }]));
    this.filtered = new Set(view.filters.map((rule) => rule.column));
    this.invalidate();
  }

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

  /** Scrolls so that the cell is in view. */
  reveal(cell: Cell): void {
    const viewHeight = this.height - HEADER_HEIGHT;
    let top = this.top;
    const y = cell.row * ROW_HEIGHT;
    if (y < top) top = y;
    else if (y + ROW_HEIGHT > top + viewHeight) top = y + ROW_HEIGHT - viewHeight;
    let left = this.left;
    const x = this.edges[cell.column] ?? 0;
    const right = this.edges[cell.column + 1] ?? x;
    const viewWidth = this.width - this.gutter;
    if (x < left) left = x;
    else if (right > left + viewWidth) left = Math.min(x, right - viewWidth);
    if (top !== this.top || left !== this.left) this.scrollTo(top, left);
  }

  /** Where the cell is on screen, relative to the grid, for an editor over it. */
  cellRect(cell: Cell): { x: number; y: number; width: number; height: number } {
    return {
      x: this.gutter + this.edges[cell.column] - this.left,
      y: HEADER_HEIGHT + cell.row * ROW_HEIGHT - this.top,
      width: this.widths[cell.column] ?? DEFAULT_WIDTH,
      height: ROW_HEIGHT,
    };
  }

  /** The value of a cell, if its rows are loaded. */
  value(cell: Cell): string | undefined {
    return this.row(cell.row)?.[cell.column];
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
    const rect = cursor ? this.cellRect(cursor) : { x: 0, y: 0, height: ROW_HEIGHT };
    this.keys.style.left = `${Math.max(0, Math.min(this.width - 2, rect.x))}px`;
    this.keys.style.top = `${Math.max(0, Math.min(this.height - rect.height, rect.y))}px`;
  }

  get maxTop(): number {
    const rows = this.source?.rows ?? 0;
    return Math.max(0, rows * ROW_HEIGHT - (this.height - HEADER_HEIGHT));
  }

  get maxLeft(): number {
    return Math.max(0, this.gutter + this.edges[this.edges.length - 1] - this.width);
  }

  get scrollTop(): number {
    return this.top;
  }

  get rowHeight(): number {
    return ROW_HEIGHT;
  }

  scrollTo(top: number, left = this.left): void {
    this.top = Math.min(Math.max(0, top), this.maxTop);
    this.left = Math.min(Math.max(0, left), this.maxLeft);
    this.place();
    this.invalidate();
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

  // Layout and scrolling

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
    this.context.font = FONT;
    this.gutter = Math.ceil(this.context.measureText(String(Math.max(rows, 1))).width) + 2 * PADDING + 4;
    this.edges = [0];
    for (const width of this.widths) this.edges.push(this.edges[this.edges.length - 1] + width);

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
    if (Math.abs(scrollTop - this.placed.top) > 1) {
      this.top = Math.min(scrollTop / this.ratioY, this.maxTop);
    }
    if (Math.abs(scrollLeft - this.placed.left) > 1) {
      this.left = Math.min(scrollLeft / this.ratioX, this.maxLeft);
    }
    this.placed = { top: scrollTop, left: scrollLeft };
    this.invalidate();
  }

  private wheel(event: WheelEvent): void {
    if (event.ctrlKey) return; // pinch to zoom
    event.preventDefault();
    const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? ROW_HEIGHT : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? this.height - HEADER_HEIGHT : 1;
    let dx = event.deltaX * unit;
    let dy = event.deltaY * unit;
    // Windows and Linux scroll sideways with Shift and the wheel.
    if (event.shiftKey && dx === 0) [dx, dy] = [dy, 0];
    this.scrollTo(this.top + dy, this.left + dx);
  }

  /** Rows a page moves: those in view, less one. */
  private get pageRows(): number {
    return Math.max(1, Math.floor((this.height - HEADER_HEIGHT) / ROW_HEIGHT) - 1);
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
    const at = this.selected?.cursor ?? { row: 0, column: 0 };
    const lastRow = rows - 1;
    const lastColumn = columns - 1;
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
        to = { row: at.row, column: jump ? lastColumn : at.column + 1 };
        break;
      case "ArrowLeft":
        to = { row: at.row, column: jump ? 0 : at.column - 1 };
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
        to = { row: at.row, column: at.column + (event.shiftKey ? -1 : 1) };
        extend = false;
        break;
      default:
        return;
    }
    event.preventDefault();
    const cell = { row: Math.max(0, Math.min(lastRow, to.row)), column: Math.max(0, Math.min(lastColumn, to.column)) };
    this.select(this.selected ? this.selected.moved(cell, extend) : Selection.at(cell));
  }

  // The pointer

  private hit(clientX: number, clientY: number, clamp = false): Hit {
    const bounds = this.scroller.getBoundingClientRect();
    let x = clientX - bounds.left;
    let y = clientY - bounds.top;
    if (clamp) {
      x = Math.max(this.gutter, Math.min(this.width - 1, x));
      y = Math.max(HEADER_HEIGHT, Math.min(this.height - 1, y));
    }
    if (x < 0 || y < 0 || x >= this.width || y >= this.height) return { area: "none" };
    const rows = this.source?.rows ?? 0;
    const row = Math.min(rows - 1, Math.floor((y - HEADER_HEIGHT + this.top) / ROW_HEIGHT));
    const column = this.columnAt(x - this.gutter + this.left);
    if (y < HEADER_HEIGHT) return x < this.gutter ? { area: "corner" } : column === null ? { area: "none" } : { area: "header", column };
    if (row < 0) return { area: "none" };
    if (x < this.gutter) return { area: "gutter", row };
    return column === null ? { area: "none" } : { area: "cell", row, column };
  }

  /** The column at `x` pixels from the left of the first column, or `null` past the last. */
  private columnAt(x: number): number | null {
    let low = 0;
    let high = this.widths.length - 1;
    if (high < 0 || x < 0 || x >= this.edges[high + 1]) return null;
    while (low < high) {
      const middle = (low + high + 1) >> 1;
      if (this.edges[middle] <= x) low = middle;
      else high = middle - 1;
    }
    return low;
  }

  private pointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    const hit = this.hit(event.clientX, event.clientY);
    if (hit.area === "none" || hit.area === "corner") return;
    event.preventDefault();
    this.focus();
    const columns = this.widths.length;
    if (hit.area === "header") {
      const bounds = this.scroller.getBoundingClientRect();
      const x = event.clientX - bounds.left;
      const icon = this.filterIconX(hit.column);
      // The icon's area reaches a little past it, as in the macOS app.
      if (this.showsFilterIcon(hit.column) && x >= icon - 3 && x <= icon + ICON + 3) this.onFilterIcon?.(hit.column);
      else this.onHeader?.(hit.column, event.shiftKey);
      return;
    }
    if (hit.area === "gutter") {
      // A row: from its last column to its first, so the cursor is at its start.
      const anchor = event.shiftKey && this.selected ? { row: this.selected.anchor.row, column: columns - 1 } : { row: hit.row, column: columns - 1 };
      this.select(new Selection(anchor, { row: hit.row, column: 0 }), false);
    } else if (hit.area === "cell") {
      const cell = { row: hit.row, column: hit.column };
      this.select(event.shiftKey && this.selected ? this.selected.moved(cell, true) : Selection.at(cell), false);
    }
    this.scroller.setPointerCapture(event.pointerId);
    this.drag = { rows: hit.area === "gutter", x: event.clientX, y: event.clientY, frame: 0 };
  }

  private pointerMove(event: PointerEvent): void {
    if (!this.drag) {
      const hit = this.hit(event.clientX, event.clientY);
      const hovered = hit.area === "header" ? hit.column : null;
      if (hovered !== this.hovered) {
        this.hovered = hovered;
        this.invalidate();
      }
      return;
    }
    this.drag.x = event.clientX;
    this.drag.y = event.clientY;
    this.extendDrag();
    if (!this.drag.frame) this.drag.frame = requestAnimationFrame(() => this.autoscroll());
  }

  private pointerUp(event: PointerEvent): void {
    if (!this.drag) return;
    cancelAnimationFrame(this.drag.frame);
    this.drag = null;
    if (this.scroller.hasPointerCapture(event.pointerId)) this.scroller.releasePointerCapture(event.pointerId);
  }

  /** Extends the selection to the cell under the pointer while dragging. */
  private extendDrag(): void {
    if (!this.drag || !this.selected) return;
    const hit = this.hit(this.drag.x, this.drag.y, true);
    if (hit.area !== "cell") return;
    const cell = this.drag.rows ? { row: hit.row, column: 0 } : { row: hit.row, column: hit.column };
    const selection = this.selected.moved(cell, true);
    if (!selection.equals(this.selected)) this.select(selection, false);
  }

  /** Scrolls while the pointer drags past an edge. */
  private autoscroll(): void {
    if (!this.drag) return;
    this.drag.frame = 0;
    const bounds = this.scroller.getBoundingClientRect();
    const speed = (overshoot: number) => Math.sign(overshoot) * Math.min(AUTOSCROLL_STEP, Math.abs(overshoot));
    const below = this.drag.y - (bounds.top + this.height - AUTOSCROLL_EDGE);
    const above = bounds.top + HEADER_HEIGHT + AUTOSCROLL_EDGE - this.drag.y;
    const right = this.drag.x - (bounds.left + this.width - AUTOSCROLL_EDGE);
    const left = bounds.left + this.gutter + AUTOSCROLL_EDGE - this.drag.x;
    const dy = below > 0 ? speed(below) : above > 0 ? -speed(above) : 0;
    const dx = this.drag.rows ? 0 : right > 0 ? speed(right) : left > 0 ? -speed(left) : 0;
    if (dx === 0 && dy === 0) return;
    this.scrollTo(this.top + dy, this.left + dx);
    this.extendDrag();
    this.drag.frame = requestAnimationFrame(() => this.autoscroll());
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

  private doubleClick(event: MouseEvent): void {
    const hit = this.hit(event.clientX, event.clientY);
    if (hit.area === "cell") this.onActivate?.({ row: hit.row, column: hit.column });
  }

  // Rows

  private row(index: number): string[] | undefined {
    return this.blocks.get(Math.floor(index / BLOCK))?.[index % BLOCK];
  }

  /** Asks for the blocks in view, and one on each side, dropping requests no longer in view. */
  private request(first: number, count: number): void {
    const rows = this.source?.rows ?? 0;
    if (count <= 0 || rows === 0) return;
    const firstBlock = Math.floor(first / BLOCK);
    const lastBlock = Math.floor((first + count - 1) / BLOCK);
    const wanted: number[] = [];
    for (let block = firstBlock - 1; block <= lastBlock + 1; block += 1) {
      if (block < 0 || block * BLOCK >= rows) continue;
      if (!this.blocks.has(block) && !this.pending.has(block)) wanted.push(block);
    }
    // Blocks in view first, then the neighbours.
    wanted.sort((a, b) => Number(a < firstBlock || a > lastBlock) - Number(b < firstBlock || b > lastBlock));
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
        .then((rows) => {
          if (generation !== this.generation) return;
          this.stats.fetch(performance.now() - started);
          this.blocks.set(block, rows);
          if (block === 0 && !this.measured) this.measure(rows);
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
    for (const block of far.slice(0, this.blocks.size - MAX_BLOCKS)) this.blocks.delete(block);
  }

  /** Fits the columns to their titles and the first rows, once per table. */
  private measure(rows: string[][]): void {
    this.measured = true;
    const columns = Math.min(this.widths.length, MEASURED_COLUMNS);
    for (let column = 0; column < columns; column += 1) {
      const values = rows.map((row) => (row[column] ?? "").slice(0, 60));
      const header = this.fit(MIN_WIDTH, HEADER_FONT, [this.title(column)]) + MARK_ROOM;
      this.widths[column] = this.fit(header, FONT, values);
    }
    this.layout();
  }

  private fit(atLeast: number, font: string, texts: string[]): number {
    this.context.font = font;
    let widest = 0;
    for (const text of texts) widest = Math.max(widest, this.context.measureText(text).width);
    return Math.min(MAX_WIDTH, Math.max(atLeast, Math.ceil(widest) + 2 * PADDING));
  }

  private isFiltered(column: number): boolean {
    return this.filtered.has(column);
  }

  private showsFilterIcon(column: number): boolean {
    return this.hovered === column || this.isFiltered(column);
  }

  /** The left of a column's filter icon, relative to the grid. */
  private filterIconX(column: number): number {
    const right = this.gutter + this.edges[column + 1] - this.left;
    return right - SORT_ICON_INSET - ICON - FILTER_ICON_GAP - ICON;
  }

  /** The area the data rows take, relative to the grid: editors stay inside it. */
  get dataArea(): { x: number; y: number; width: number; height: number } {
    return { x: this.gutter, y: HEADER_HEIGHT, width: this.width - this.gutter, height: this.height - HEADER_HEIGHT };
  }

  /** The header's title of a column, or its letter as in Excel. */
  title(column: number): string {
    const title = this.source?.header[column];
    if (title !== undefined) return title;
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
      line: color("--grid-line"),
      header: color("--grid-header"),
      headerText: color("--grid-header-text"),
      placeholder: color("--grid-placeholder"),
      selection: color("--grid-selection"),
      cursor: color("--accent"),
      selectedHeader: color("--grid-selected-header"),
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

  private draw(): boolean {
    if ((window.devicePixelRatio || 1) !== this.dpr) this.resize();
    const started = performance.now();
    const { context: g, colors, width, height, gutter, dpr } = this;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = colors.background;
    g.fillRect(0, 0, width, height);
    const source = this.source;
    if (!source) return true;

    const viewHeight = height - HEADER_HEIGHT;
    const first = Math.floor(this.top / ROW_HEIGHT);
    const shift = HEADER_HEIGHT + first * ROW_HEIGHT - this.top;
    const count = Math.max(0, Math.min(source.rows - first, Math.ceil((viewHeight - (shift - HEADER_HEIGHT)) / ROW_HEIGHT)));
    this.request(first, count);

    // Columns in view: the first one whose right edge passes the left of the view.
    let firstColumn = 0;
    while (firstColumn < this.widths.length - 1 && this.edges[firstColumn + 1] <= this.left) firstColumn += 1;
    let lastColumn = firstColumn;
    while (lastColumn < this.widths.length - 1 && gutter + this.edges[lastColumn + 1] - this.left < width) lastColumn += 1;
    const columnX = (column: number) => gutter + this.edges[column] - this.left;
    // Lines fall on device pixels.
    const crisp = (value: number) => (Math.round(value * dpr) + 0.5) / dpr;

    g.save();
    g.beginPath();
    g.rect(0, HEADER_HEIGHT, width, viewHeight);
    g.clip();
    let complete = true;
    for (let i = 0; i < count; i += 1) {
      const index = first + i;
      const y = shift + i * ROW_HEIGHT;
      if (index % 2 === 1) {
        g.fillStyle = colors.alternate;
        g.fillRect(0, y, width, ROW_HEIGHT);
      }
      if (!this.row(index)) {
        complete = false;
        g.fillStyle = colors.placeholder;
        for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
          g.fillRect(columnX(column) + PADDING, y + 8, Math.min(this.widths[column] - 2 * PADDING, 48), ROW_HEIGHT - 16);
        }
      }
    }

    // The selection, under the text.
    const selection = this.selected?.range;
    if (selection && selection.bottom >= first && selection.top < first + count) {
      const x = columnX(selection.left);
      const right = columnX(selection.right) + this.widths[selection.right];
      const y = shift + (selection.top - first) * ROW_HEIGHT;
      const bottom = shift + (selection.bottom - first + 1) * ROW_HEIGHT;
      g.fillStyle = colors.selection;
      g.fillRect(x, y, right - x, bottom - y);
    }

    g.font = FONT;
    g.textBaseline = "middle";
    g.fillStyle = colors.text;
    for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
      const x = columnX(column);
      const cellWidth = this.widths[column];
      // Glyphs are at least about 4 px wide, so longer text would be cut off anyway.
      const characters = Math.ceil(cellWidth / 4) + 2;
      g.save();
      g.beginPath();
      g.rect(x, HEADER_HEIGHT, cellWidth - PADDING / 2, viewHeight);
      g.clip();
      for (let i = 0; i < count; i += 1) {
        let text = this.row(first + i)?.[column];
        if (!text) continue;
        if (text.length > characters) text = text.slice(0, characters);
        if (text.includes("\n") || text.includes("\r")) text = text.replace(/\r\n|\r|\n/g, " ⏎ ");
        g.fillText(text, x + PADDING, shift + i * ROW_HEIGHT + ROW_HEIGHT / 2);
      }
      g.restore();
    }

    g.strokeStyle = colors.line;
    g.lineWidth = 1 / dpr;
    g.beginPath();
    for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
      const x = crisp(columnX(column) + this.widths[column]);
      g.moveTo(x, HEADER_HEIGHT);
      g.lineTo(x, height);
    }
    g.stroke();
    // The active cell.
    const cursor = this.selected?.cursor;
    if (cursor && cursor.row >= first && cursor.row < first + count) {
      g.strokeStyle = colors.cursor;
      g.lineWidth = 2;
      g.strokeRect(columnX(cursor.column) + 1, shift + (cursor.row - first) * ROW_HEIGHT + 1, this.widths[cursor.column] - 2, ROW_HEIGHT - 2);
    }
    g.restore();

    // Row numbers.
    g.fillStyle = colors.header;
    g.fillRect(0, HEADER_HEIGHT, gutter, viewHeight);
    if (selection && selection.bottom >= first && selection.top < first + count) {
      const y = Math.max(HEADER_HEIGHT, shift + (selection.top - first) * ROW_HEIGHT);
      const bottom = Math.min(height, shift + (selection.bottom - first + 1) * ROW_HEIGHT);
      g.fillStyle = colors.selectedHeader;
      g.fillRect(0, y, gutter, bottom - y);
    }
    g.save();
    g.beginPath();
    g.rect(0, HEADER_HEIGHT, gutter, viewHeight);
    g.clip();
    g.fillStyle = colors.muted;
    g.textAlign = "right";
    for (let i = 0; i < count; i += 1) {
      g.fillText(String(first + i + 1), gutter - PADDING, shift + i * ROW_HEIGHT + ROW_HEIGHT / 2);
    }
    g.restore();

    // Column titles.
    g.fillStyle = colors.header;
    g.fillRect(0, 0, width, HEADER_HEIGHT);
    if (selection) {
      const x = Math.max(gutter, columnX(selection.left));
      const right = columnX(selection.right) + this.widths[selection.right];
      g.fillStyle = colors.selectedHeader;
      if (right > x) g.fillRect(x, 0, right - x, HEADER_HEIGHT);
    }
    // The column of the first sort key stands out, as NSTableView's highlighted column.
    const highlighted = [...this.sorted].find(([, key]) => key.order === 0)?.[0];
    if (highlighted !== undefined && highlighted < this.widths.length) {
      const x = Math.max(gutter, columnX(highlighted));
      const right = columnX(highlighted) + this.widths[highlighted];
      g.fillStyle = colors.selectedHeader;
      if (right > x) g.fillRect(x, 0, right - x, HEADER_HEIGHT);
    }
    g.font = HEADER_FONT;
    g.fillStyle = colors.headerText;
    g.textAlign = "left";
    g.beginPath();
    for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
      const x = columnX(column);
      if (x + this.widths[column] < gutter) continue;
      // Icons at the right: sort when sorted or hovered, filter when filtered or hovered.
      const sort = this.sorted.get(column);
      const showsSort = !!sort || this.hovered === column;
      const showsFilter = this.showsFilterIcon(column);
      const right = x + this.widths[column];
      const iconsWidth = showsFilter ? SORT_ICON_INSET + 2 * ICON + FILTER_ICON_GAP : showsSort ? SORT_ICON_INSET + ICON : 0;
      g.save();
      g.beginPath();
      g.rect(Math.max(x, gutter), 0, Math.max(0, this.widths[column] - PADDING / 2 - iconsWidth), HEADER_HEIGHT);
      g.clip();
      g.fillStyle = colors.headerText;
      g.fillText(this.title(column), x + PADDING, HEADER_HEIGHT / 2);
      g.restore();
      if (showsSort) this.drawSortIcon(right - SORT_ICON_INSET - ICON, sort, sort ? colors.cursor : colors.muted);
      if (showsFilter) this.drawFilterIcon(this.filterIconX(column), this.isFiltered(column) ? colors.cursor : colors.muted);
    }
    g.fillStyle = colors.header;
    g.fillRect(0, 0, gutter, HEADER_HEIGHT);

    g.strokeStyle = colors.line;
    g.beginPath();
    g.moveTo(0, crisp(HEADER_HEIGHT - 1));
    g.lineTo(width, crisp(HEADER_HEIGHT - 1));
    g.moveTo(crisp(gutter - 1), 0);
    g.lineTo(crisp(gutter - 1), height);
    for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
      const x = crisp(columnX(column) + this.widths[column]);
      if (x < gutter) continue;
      g.moveTo(x, 0);
      g.lineTo(x, HEADER_HEIGHT);
    }
    g.stroke();

    this.stats.draw(performance.now() - started);
    this.placeKeys();
    for (const listener of this.afterDraw) listener();
    this.onFrame?.(complete);
    return complete;
  }
}
