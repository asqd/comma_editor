// The editor of one cell, over it in the grid, as the field editor of the macOS app
// (GridViewController.editActiveCell and controlTextDidEndEditing).

import type { Grid } from "./Grid";
import type { Cell } from "./selection";

/** Where the cursor goes after a value is written: down after Return, aside after Tab. */
export type Move = "down" | "right" | "left" | null;

export class CellEditor {
  private readonly input: HTMLInputElement;
  private cell: Cell | null = null;
  private original = "";
  /** Set while the editor itself ends editing, so that losing focus does not end it again. */
  private ending = false;

  /**
   * @param commit writes `value`, or only moves the cursor when the value is `null`, unchanged.
   */
  constructor(
    private readonly grid: Grid,
    host: HTMLElement,
    private readonly commit: (cell: Cell, value: string | null, move: Move) => void,
  ) {
    this.input = document.createElement("input");
    this.input.className = "cell-editor";
    // Values are data: no corrections, completions or capitals the user did not type.
    this.input.autocomplete = "off";
    this.input.spellcheck = false;
    this.input.setAttribute("autocorrect", "off");
    this.input.setAttribute("autocapitalize", "off");
    this.input.hidden = true;
    host.append(this.input);
    this.input.addEventListener("keydown", (event) => this.key(event));
    this.input.addEventListener("blur", () => {
      if (!this.ending) this.end(null);
    });
    grid.afterDraw.push(() => this.place());
  }

  get editing(): boolean {
    return this.cell !== null;
  }

  /** Edits `cell`, starting from `value` all selected, or from what was typed. */
  start(cell: Cell, value: string, typed?: string): void {
    this.cell = cell;
    this.original = value;
    this.input.value = typed ?? value;
    this.input.hidden = false;
    this.place();
    this.input.focus();
    if (typed === undefined) this.input.select();
    else this.input.setSelectionRange(this.input.value.length, this.input.value.length);
  }

  /** Ends editing, writing the value if it changed. */
  end(move: Move): void {
    const cell = this.cell;
    if (!cell) return;
    const value = this.input.value;
    this.close();
    this.commit(cell, value === this.original ? null : value, move);
  }

  cancel(): void {
    this.close();
  }

  private close(): void {
    this.ending = true;
    this.cell = null;
    this.input.hidden = true;
    this.grid.focus();
    this.ending = false;
  }

  private key(event: KeyboardEvent): void {
    // The grid's own keys stay out of the editor.
    event.stopPropagation();
    if (event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey && !event.altKey && !event.metaKey && !event.ctrlKey) {
      event.preventDefault();
      this.end("down");
    } else if (event.key === "Tab") {
      event.preventDefault();
      this.end(event.shiftKey ? "left" : "right");
    } else if (event.key === "Escape") {
      event.preventDefault();
      this.cancel();
    }
  }

  /** Keeps the editor over its cell as the grid scrolls, within the data rows. */
  private place(): void {
    if (!this.cell) return;
    const cell = this.grid.cellRect(this.cell);
    const area = this.grid.dataArea;
    const left = Math.max(cell.x, area.x);
    const top = Math.max(cell.y, area.y);
    const right = Math.min(cell.x + cell.width, area.x + area.width);
    const bottom = Math.min(cell.y + cell.height, area.y + area.height);
    const visible = right > left && bottom > top;
    this.input.style.visibility = visible ? "visible" : "hidden";
    Object.assign(this.input.style, {
      left: `${cell.x}px`,
      top: `${cell.y}px`,
      width: `${Math.max(cell.width, 80)}px`,
      height: `${cell.height}px`,
      clipPath: `inset(${top - cell.y}px ${cell.x + Math.max(cell.width, 80) - right}px ${cell.y + cell.height - bottom}px ${left - cell.x}px)`,
    });
  }
}
