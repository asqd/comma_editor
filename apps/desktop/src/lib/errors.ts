// What went wrong, in the words of the macOS app (CSVDocument.userFacingError).

import { CoreFailure } from "./api";
import { describeFormula, type FormulaProblem } from "./formula";
import { L, count } from "./i18n";
import { columnLetter } from "./workbook";

export interface Problem {
  message: string;
  detail?: string;
}

type Detail = Record<string, Record<string, unknown> | undefined> | string;

export function describe(error: unknown): Problem {
  const detail = (error instanceof CoreFailure ? error.detail : error) as Detail;
  if (typeof detail === "string") {
    if (detail === "lastColumn") return { message: L("The last column cannot be deleted.") };
    if (detail === "busy")
      return {
        message: L("The file is still being read."),
        detail: L("Changes and saving are possible once the whole file is read; the progress is in the status bar."),
      };
    if (detail === "editing")
      return { message: L("Another change is being made."), detail: L("Wait until it ends or cancel it in the status bar.") };
    return { message: detail };
  }
  if (!detail || typeof detail !== "object") return { message: String(error) };
  // A message rather than an error of the core, in the words of the macOS app where it has them.
  if (typeof (detail as Record<string, unknown>).message === "string") return { message: L(String((detail as Record<string, unknown>).message)) };
  const [kind, fields = {}] = Object.entries(detail)[0] ?? ["", {}];
  const text = (name: string) => String((fields as Record<string, unknown>)[name] ?? "");
  const number = (name: string) => Number((fields as Record<string, unknown>)[name] ?? 0);
  switch (kind) {
    case "io":
      return { message: L("The file cannot be read or written."), detail: text("message") };
    case "outOfRange":
      return { message: L("There is no such row or column."), detail: text("message") };
    case "unmappable":
      return {
        message: L("The value in row %lld, column %lld cannot be saved in %@.", number("row") + 1, number("column") + 1, text("encoding")),
        detail: L("Replace the characters that this encoding cannot represent."),
      };
    case "invalidDelimiter":
      return {
        message: L("“%@” cannot be used as a delimiter.", text("delimiter")),
        detail: L("A delimiter is one character other than a quote or a line break."),
      };
    case "invalidPattern":
      return { message: L("The regular expression is not valid."), detail: text("message") };
    case "recipe": {
      const column = (fields as Record<string, unknown>).column;
      return {
        message: L("Step %lld of the recipe failed. The table did not change.", number("step") + 1),
        detail:
          typeof column === "string"
            ? L(
                "This file has no column “%@”. Recipes find columns by their names; rename the column or remove the step in Recipes ▸ Manage Recipes.",
                column,
              )
            : text("message"),
      };
    }
    case "invalidRecipe":
      return { message: L("The recipe file cannot be read."), detail: text("message") };
    case "export":
      return { message: L("The table cannot be exported."), detail: text("message") };
    case "excelRowLimit":
      return {
        message: L("The table does not fit on an Excel sheet."),
        detail: L(
          "An Excel sheet holds at most %@ rows, and the table has %@. Filter the rows to export fewer of them.",
          count(number("limit")),
          count(number("rows")),
        ),
      };
    case "formula":
      return { message: describeFormula((fields as { problem: FormulaProblem }).problem) };
    case "workbook":
      return { message: L("The Excel workbook cannot be read."), detail: describeWorkbook((fields as { problem: unknown }).problem) };
    case "sheetStructure":
      return describeSheetChange(text("change"));
    case "mergedCell":
      return {
        message: L("This cell is inside a merged area."),
        detail: L("A merged area shows one value, kept in its first cell, %@. Type the value there.", `${columnLetter(number("column"))}${number("row") + 1}`),
      };
    default:
      return { message: JSON.stringify(detail) };
  }
}

/** Why a workbook cannot be read (CSVDocument.describe in the macOS app). */
function describeWorkbook(problem: unknown): string {
  const [kind, fields = {}] = typeof problem === "string" ? [problem, {}] : (Object.entries(problem as object)[0] ?? ["", {}]);
  const field = (name: string) => (fields as Record<string, unknown>)[name];
  switch (kind) {
    case "notAWorkbook":
      return L("The file is not an Excel workbook, or it is damaged.");
    case "webPage":
      return L("The file is a web page with a table, saved with an Excel extension, as many exports are. Open it in Excel and save it as .xlsx.");
    case "xml2003":
      return L("The file is an Excel 2003 XML spreadsheet. Open it in Excel and save it as .xlsx.");
    case "encrypted":
      return L("The workbook is protected with a password. Open it in Excel or Numbers and save it without a password.");
    case "damaged":
      // What the XML reader found is technical and in English; where it is helps more.
      return field("position") === null || field("position") === undefined
        ? L("A part of the file is damaged: %@. Excel may be able to repair it when it opens the file.", String(field("part")))
        : L("A part of the file is damaged: %@, byte %@. Excel may be able to repair it when it opens the file.", String(field("part")), count(Number(field("position"))));
    case "noSuchSheet":
      return L("The workbook has no sheet %@.", count(Number(field("sheet"))));
    case "notATable":
      return L("“%@” is a chart or macro sheet: it has no cells to show.", String(field("sheet")));
    case "missingCells":
      return L("The cells of sheet “%@” are missing from the file.", String(field("sheet")));
    case "readOnly":
      return L("Excel 97–2003 workbooks can only be read. Export the sheet as XLSX or CSV to change it.");
    default:
      return JSON.stringify(problem);
  }
}

/** Why rows or columns of a sheet cannot change. */
function describeSheetChange(change: string): Problem {
  switch (change) {
    case "rows":
      return {
        message: L("Rows of this sheet cannot be inserted, deleted or moved."),
        detail: L("Formulas, merged cells or other parts of the workbook refer to them, and this command would leave the references behind. Rows can be added at the end."),
      };
    case "columns":
      return {
        message: L("Columns of a workbook cannot be inserted, deleted or moved yet."),
        detail: L("Values in the columns can be changed. To change the columns, export the sheet as CSV."),
      };
    case "pivotTable":
      return {
        message: L("A pivot table is laid out on this sheet."),
        detail: L("Its rows and columns stay where they are, as the pivot table takes them. Rows can be added at the end."),
      };
    case "tableRowsMoved":
      return {
        message: L("These rows would break an Excel table."),
        detail: L(
          "Rows of a table move and sort among its data rows, without its headers and totals, and with nothing beside the table in them; other rows move outside tables. Rows can always be inserted and deleted.",
        ),
      };
    case "tableEdges":
      return {
        message: L("These rows of an Excel table cannot be deleted."),
        detail: L("The header row, the totals row and the last data row hold the table together. Other rows of it can be deleted."),
      };
    case "tableColumns":
      return { message: L("Every column of an Excel table cannot be deleted."), detail: L("A table keeps at least one column. To remove the table, delete it in Excel.") };
    case "tableColumnInUse":
      return { message: L("Formulas read this column of an Excel table."), detail: L("Change the formulas that name it first, so that they do not lose what they read.") };
    case "mergedArea":
      return {
        message: L("This would split merged cells."),
        detail: L("A merged area moves whole, its rows and columns side by side. Unmerge it in Excel, or move all of it together."),
      };
    case "tableColumnsMoved":
      return {
        message: L("Columns of an Excel table cannot be moved."),
        detail: L("Columns outside tables move, references following them. To reorder a table's columns, do it in Excel."),
      };
    default:
      return { message: L("Rows of this sheet cannot be inserted, deleted or moved.") };
  }
}

/** The problem as one line, for the status bar. */
export function sentence(error: unknown): string {
  const problem = describe(error);
  return problem.detail ? `${problem.message} ${problem.detail}` : problem.message;
}
