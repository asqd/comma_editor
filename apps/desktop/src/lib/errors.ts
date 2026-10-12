// What went wrong, in the words of the macOS app (CSVDocument.userFacingError).

import { CoreFailure } from "./api";
import { L, count } from "./i18n";

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
  // A message rather than an error of the core.
  if (typeof (detail as Record<string, unknown>).message === "string") return { message: String((detail as Record<string, unknown>).message) };
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
    default:
      return { message: JSON.stringify(detail) };
  }
}

/** The problem as one line, for the status bar. */
export function sentence(error: unknown): string {
  const problem = describe(error);
  return problem.detail ? `${problem.message} ${problem.detail}` : problem.message;
}
