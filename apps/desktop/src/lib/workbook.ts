// Excel workbooks as the macOS app shows them (WorkbookDocument and GridViewController+Sheets):
// numbers and dates as this system shows them, formulas as Excel writes them here, and the
// widths Excel gives columns.

import type { SheetLayout } from "./api";
import { L, russian } from "./i18n";

/** comma-ffi's DisplayLocale: how this system shows numbers and dates, which Excel follows. */
export interface DisplayLocale {
  decimalSeparator: string;
  groupSeparator: string;
  shortDate: string;
  monthNames: string[];
  monthAbbreviations: string[];
  dayNames: string[];
  dayAbbreviations: string[];
  am: string;
  pm: string;
  trueText: string;
  falseText: string;
}

/** The system's region, which numbers and dates follow, whatever the interface's language. */
const region = new Intl.NumberFormat().resolvedOptions().locale;

export function displayLocale(): DisplayLocale {
  const parts = new Intl.NumberFormat(region).formatToParts(1234567.5);
  const part = (type: string, fallback: string) => parts.find((item) => item.type === type)?.value ?? fallback;
  const dates = (options: Intl.DateTimeFormatOptions, count: number, day: (index: number) => Date) =>
    Array.from({ length: count }, (_, index) => new Intl.DateTimeFormat(region, options).format(day(index)));
  const month = (index: number) => new Date(2026, index, 1);
  // 4 January 2026 was a Sunday, the first day of Excel's week.
  const weekday = (index: number) => new Date(2026, 0, 4 + index);
  const periods = new Intl.DateTimeFormat(region, { hour: "numeric", hour12: true });
  const period = (hour: number) => periods.formatToParts(new Date(2026, 0, 1, hour)).find((item) => item.type === "dayPeriod")?.value ?? "";
  return {
    decimalSeparator: part("decimal", "."),
    // A no-break space groups digits in many regions; Excel writes it as one.
    groupSeparator: part("group", ","),
    shortDate: shortDateCode(),
    monthNames: dates({ month: "long" }, 12, month),
    monthAbbreviations: dates({ month: "short" }, 12, month).map((name) => name.replace(/\.$/, "")),
    dayNames: dates({ weekday: "long" }, 7, weekday),
    dayAbbreviations: dates({ weekday: "short" }, 7, weekday),
    am: period(9) || "AM",
    pm: period(21) || "PM",
    trueText: L("TRUE"),
    falseText: L("FALSE"),
  };
}

/** The system's short date as an Excel format code, such as `dd.mm.yyyy` or `m/d/yyyy`. */
function shortDateCode(): string {
  // 5 March 2026: one digit for the day and the month tells `d` from `dd`.
  const parts = new Intl.DateTimeFormat(region, { year: "numeric", month: "numeric", day: "numeric" }).formatToParts(new Date(2026, 2, 5));
  return parts
    .map((item) => {
      if (item.type === "day") return item.value.length > 1 ? "dd" : "d";
      if (item.type === "month") return item.value.length > 1 ? "mm" : "m";
      if (item.type === "year") return "yyyy";
      return item.value.replace(/[a-zA-Z]+/g, (text) => `"${text}"`);
    })
    .join("");
}

/** What a workbook needs to open: the locale, how formulas are shown, and the time zone. */
export function workbookLocale() {
  return {
    display: displayLocale(),
    formulas: {
      // As Excel shows formulas where numbers have a decimal comma: `;` between arguments.
      comma: new Intl.NumberFormat(region).formatToParts(1.5).find((item) => item.type === "decimal")?.value === ",",
      russian,
    },
    utcOffset: -new Date().getTimezoneOffset() * 60,
  };
}

/** An Excel workbook by its name; the core also knows one by its first bytes. */
export const isWorkbookName = (path: string) => /\.(xlsx|xlsm|xls)$/i.test(path);

/**
 * Excel measures widths in digits of its default font, Calibri 11; the table's font is larger,
 * and its cells have more padding (GridViewController.points(forExcelWidth:)).
 */
export function pixelsForExcelWidth(width: number): number {
  const pixels = width < 1 ? width * 12 : width * 7 + 5;
  return Math.min(800, Math.max(36, Math.round(pixels * 1.35)));
}

/** Widths and hidden columns as Excel shows a sheet of `columns` columns. */
export function sheetColumns(layout: SheetLayout | null, columns: number, standard: number): { widths: number[]; hidden: number[] } {
  const widths = Array.from({ length: columns }, () => standard);
  const hidden = new Set<number>();
  if (!layout) return { widths, hidden: [] };
  // Widths only when the sheet sets some; otherwise the table's own default.
  if (layout.columns.some((column) => column.width !== null) || layout.defaultColumnWidth !== null) {
    widths.fill(pixelsForExcelWidth(layout.defaultColumnWidth ?? 8.43));
  }
  for (const column of layout.columns) {
    for (let index = column.first; index <= Math.min(column.last, columns - 1); index += 1) {
      if (column.width !== null) widths[index] = pixelsForExcelWidth(column.width);
      if (column.hidden) hidden.add(index);
    }
  }
  // At least one column stays in view.
  return { widths, hidden: hidden.size >= columns ? [] : [...hidden] };
}

/** A column's letter, as Excel names it: A, B, …, Z, AA. */
export function columnLetter(column: number): string {
  let name = "";
  for (let n = column + 1; n > 0; n = Math.floor((n - 1) / 26)) name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
  return name;
}
