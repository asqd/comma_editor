// The table: a canvas that draws only the cells in view, over a scroller whose scrollbars stand
// for the whole table. Rows come from the core in blocks, as the view reaches them.
//
// A table of 60 million rows is 1.4 billion pixels tall, far past the largest element a
// browser lays out (about 33 million px in Chromium, less in WebKit). So the scroll position
// lives here, in pixels of the table, and the scroller's range is that position scaled down:
// wheel and keys move the table by their own pixels, the scrollbar thumb by its share.

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

interface Colors {
  background: string;
  alternate: string;
  text: string;
  muted: string;
  line: string;
  header: string;
  headerText: string;
  placeholder: string;
}

export class Grid {
  readonly stats = new Stats();
  /** Called after each frame with whether every row in view had its data. */
  onFrame: ((complete: boolean) => void) | null = null;

  private readonly scroller: HTMLDivElement;
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

  private readonly blocks = new Map<number, string[][]>();
  private readonly pending = new Set<number>();
  private queue: number[] = [];
  private inFlight = 0;
  private frame = 0;

  constructor(private readonly host: HTMLElement) {
    this.scroller = document.createElement("div");
    this.scroller.className = "grid-scroller";
    this.scroller.tabIndex = 0;
    this.spacer = document.createElement("div");
    this.spacer.className = "grid-spacer";
    this.scroller.append(this.spacer);
    this.canvas = document.createElement("canvas");
    this.canvas.className = "grid-canvas";
    // The scroller lies over the canvas, transparent, so that overlay scrollbars stay visible.
    host.append(this.canvas, this.scroller);
    const context = this.canvas.getContext("2d", { alpha: false });
    if (!context) throw new Error("no 2D canvas");
    this.context = context;
    this.colors = this.readColors();

    this.scroller.addEventListener("scroll", () => this.scrolled());
    this.scroller.addEventListener("wheel", (event) => this.wheel(event), { passive: false });
    this.scroller.addEventListener("keydown", (event) => this.key(event));
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
    this.widths = Array.from({ length: source?.columns ?? 0 }, (_, column) =>
      column < MEASURED_COLUMNS ? this.fit(MIN_WIDTH, HEADER_FONT, [this.title(column)]) : DEFAULT_WIDTH,
    );
    this.layout();
    this.invalidate();
    this.scroller.focus();
  }

  /** The table grew: more rows were indexed, or columns appeared. */
  update(): void {
    const columns = this.source?.columns ?? 0;
    while (this.widths.length < columns) this.widths.push(DEFAULT_WIDTH);
    // The last block may have been cut short by the end of what was indexed then.
    for (const [index, rows] of this.blocks) if (rows.length < BLOCK) this.blocks.delete(index);
    this.layout();
    this.invalidate();
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

  private key(event: KeyboardEvent): void {
    const page = this.height - HEADER_HEIGHT - ROW_HEIGHT;
    const command = event.ctrlKey || event.metaKey;
    const moves: Record<string, () => [number, number]> = {
      ArrowDown: () => [command ? this.maxTop : this.top + ROW_HEIGHT, this.left],
      ArrowUp: () => [command ? 0 : this.top - ROW_HEIGHT, this.left],
      ArrowRight: () => [this.top, this.left + 40],
      ArrowLeft: () => [this.top, this.left - 40],
      PageDown: () => [this.top + page, this.left],
      PageUp: () => [this.top - page, this.left],
      Home: () => (command ? [0, this.left] : [this.top, 0]),
      End: () => (command ? [this.maxTop, this.left] : [this.top, this.maxLeft]),
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    this.scrollTo(...move());
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
      const header = this.fit(MIN_WIDTH, HEADER_FONT, [this.title(column)]);
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

  /** The header's title of a column, or its letter as in Excel. */
  private title(column: number): string {
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
    };
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
    g.restore();

    // Row numbers.
    g.fillStyle = colors.header;
    g.fillRect(0, HEADER_HEIGHT, gutter, viewHeight);
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
    g.font = HEADER_FONT;
    g.fillStyle = colors.headerText;
    g.textAlign = "left";
    g.beginPath();
    for (let column = firstColumn; column <= lastColumn && this.widths.length > 0; column += 1) {
      const x = columnX(column);
      if (x + this.widths[column] < gutter) continue;
      g.save();
      g.beginPath();
      g.rect(Math.max(x, gutter), 0, this.widths[column] - PADDING / 2, HEADER_HEIGHT);
      g.clip();
      g.fillText(this.title(column), x + PADDING, HEADER_HEIGHT / 2);
      g.restore();
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
    this.onFrame?.(complete);
    return complete;
  }
}
