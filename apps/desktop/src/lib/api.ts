// Calls to the core: through Tauri in the app, through the development bridge in a browser
// (`comma-bridge`, behind the Vite proxy). Both run `Sessions::dispatch` in
// src-tauri/src/session.rs; the types here mirror its replies and comma-ffi's records.

import { invoke } from "@tauri-apps/api/core";

export const inTauri = "__TAURI_INTERNALS__" in window;

/** The bridge keeps a document per browser tab, as the app does per window. */
const tab = (() => {
  try {
    const id = sessionStorage.getItem("comma-window") ?? `tab-${crypto.randomUUID()}`;
    sessionStorage.setItem("comma-window", id);
    return id;
  } catch {
    return `tab-${Math.random().toString(36).slice(2)}`;
  }
})();

/** A command failed: `detail` is an error of the core (see `describe` in errors.ts) or a message. */
export class CoreFailure extends Error {
  constructor(readonly detail: unknown) {
    super(typeof detail === "string" ? detail : JSON.stringify(detail));
  }
}

export async function call<T>(command: string, args: object = {}): Promise<T> {
  if (inTauri) {
    try {
      return await invoke<T>("core", { command, args });
    } catch (error) {
      throw new CoreFailure(error);
    }
  }
  const response = await fetch("/core", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ window: tab, command, args }),
  });
  const body: unknown = await response.json().catch(() => ({ message: `bridge: HTTP ${response.status}` }));
  if (!response.ok) throw new CoreFailure(body);
  return body as T;
}

export type TextEncoding =
  | "utf8"
  | "utf16Le"
  | "utf16Be"
  | "windows1251"
  | "koi8R"
  | "windows1252"
  | "windows1250"
  | "windows1253"
  | "windows1254"
  | "windows1257"
  | "koi8U"
  | "ibm866";

export type LineEnding = "lf" | "crLf" | "cr";

export interface SortKey {
  column: number;
  ascending: boolean;
}

export type FilterOp =
  | "contains"
  | "notContains"
  | "equals"
  | "notEquals"
  | "startsWith"
  | "endsWith"
  | "matches"
  | "isEmpty"
  | "isNotEmpty"
  | "greaterThan"
  | "lessThan";

export interface FilterRule {
  /** `null` means any column. */
  column: number | null;
  op: FilterOp;
  value: string;
  caseSensitive: boolean;
}

export interface ViewSpec {
  sort: SortKey[];
  filters: FilterRule[];
}

/** Visible rows `top..=bottom` and columns `left..=right`. */
export interface CellRange {
  top: number;
  left: number;
  bottom: number;
  right: number;
}

export interface CellPosition {
  row: number;
  column: number;
}

export interface SearchQuery {
  text: string;
  caseSensitive: boolean;
  wholeCell: boolean;
  regex: boolean;
  column: number | null;
  /** Search these visible cells only. */
  range: CellRange | null;
}

export interface SaveFormat {
  encoding: TextEncoding;
  bom: boolean;
  delimiter: string;
  lineEnding: LineEnding;
}

export interface Status {
  path: string | null;
  /** Data rows shown by the sort and filters. */
  rows: number;
  dataRows: number;
  columns: number;
  /** Column titles; empty when the first row is data. */
  header: string[];
  hasHeader: boolean;
  encoding: string;
  hasBom: boolean;
  delimiter: string;
  lineEnding: LineEnding;
  detectionConfident: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  /** Changes whenever the rows to show change. */
  change: number | null;
  view: ViewSpec;
  /** Share of a large file read so far, while it is read. */
  indexing: number | null;
  job: { kind: "view" | "edit"; fraction: number | null } | null;
  /** Steps recorded so far while a recipe is recorded. */
  recording: number | null;
  /** A sheet of an Excel workbook: rows numbered as Excel numbers them, cells with formulas. */
  isSheet: boolean;
  /** Bumps when formulas computed again changed what the table shows. */
  revision: number;
  /** The workbook, for a sheet of one. */
  book: BookStatus | null;
}

export interface SheetInfo {
  name: string;
  visibility: "visible" | "hidden" | "veryHidden";
  /** A sheet of cells; charts and old macro sheets are not. */
  isTable: boolean;
}

/** An Excel workbook open in the window (WorkbookDocument in the macOS app). */
export interface BookStatus {
  sheets: SheetInfo[];
  /** The sheet shown. */
  index: number;
  /** A sheet being read, and the share read. */
  loading: { index: number; fraction: number } | null;
  /** Excel 97–2003, which is only read. */
  legacy: boolean;
  /** The first row of every sheet names its columns. */
  csvMode: boolean;
  /** Formulas that read themselves, and ones Excel computes when it opens the file. */
  circular: number;
  notComputed: number;
  /** Rows others refer to: they move only through the workbook, and not at all when locked. */
  rowsFixed: boolean;
  /** Tables, drawings or pivot tables hold the rows and columns where they are. */
  structureLocked: boolean;
  /** Why reading a sheet or computing formulas failed, once. */
  problem: unknown;
}

/** Columns `first..=last` as a sheet sets them; widths in characters of Excel's default font. */
export interface SheetColumns {
  first: number;
  last: number;
  width: number | null;
  hidden: boolean;
}

export interface SheetLayout {
  merged: { top: number; left: number; bottom: number; right: number }[];
  columns: SheetColumns[];
  defaultColumnWidth: number | null;
  /** Rows of the sheet frozen at the top, its first row counted. */
  frozenRows: number;
  frozenColumns: number;
}

export interface RangeStats {
  cells: number;
  filled: number;
  numbers: number;
  sum: number;
}

export interface EditResult {
  changed: boolean;
  count: number;
  /** For formulas: rows computed and those that failed. */
  report: import("./formula").FormulaReport | null;
}

export interface FilePreview {
  encoding: TextEncoding;
  delimiter: string;
  hasHeader: boolean;
  rows: string[][];
  columnCount: number;
  detectionConfident: boolean;
}

export type ValueKind = "empty" | "integer" | "decimal" | "date" | "boolean" | "text";

export interface ColumnProfile {
  kind: ValueKind;
  total: number;
  empty: number;
  unique: number;
  /** Values not of the column's kind. */
  mismatched: number;
  /** Values with spaces around them. */
  padded: number;
  min: string | null;
  max: string | null;
  mean: number | null;
  median: number | null;
  minLength: number;
  maxLength: number;
  topValues: { value: string; count: number }[];
  histogram: number[];
}

export type ProblemKind = "unclosedQuote" | "strayQuote" | "missingFields" | "extraFields";

export interface Problem {
  /** A row of the file, the header counted. */
  row: number;
  kind: ProblemKind;
  fields: number;
  /** Lines of the file the row takes. */
  lines: number;
}

export interface RepairReport {
  /** The number of fields most rows have. */
  expectedFields: number;
  /** The first problems in file order. */
  problems: Problem[];
  counts: { kind: ProblemKind; count: number }[];
  /** Values in the first rows that look like text read in the wrong encoding. */
  garbledSample: number;
}

export type RepairFix =
  | "quoteAsText"
  | "removeQuote"
  | "padFields"
  | "joinNext"
  | { mergeFields: { column: number } }
  | "dropEmptyFields"
  | "deleteRow";

export interface RepairOption {
  fix: RepairFix;
  /** The rows as the fix leaves them. */
  rows: string[][];
  rowCount: number;
}
