// The formula language as people read it (FormulaHelp in the macOS app): its functions, and
// its problems in words.

import { L } from "./i18n";

export type FormulaErrorKind =
  | "unexpectedEnd"
  | "unexpectedText"
  | "unclosedText"
  | "unclosedColumn"
  | "unknownColumn"
  | "unknownFunction"
  | "wrongArgumentCount"
  | "notANumber"
  | "notADate"
  | "divisionByZero";

export interface FormulaProblem {
  kind: FormulaErrorKind;
  position: number;
  detail: string;
}

export interface FormulaReport {
  rows: number;
  errors: number;
  /** The first failing row of the file, and why. */
  firstErrorRow: number | null;
  firstError: FormulaProblem | null;
}

export const functions = (): { name: string; signature: string; summary: string }[] => [
  { name: "IF", signature: L("IF(condition, then, else)"), summary: L("one of two values") },
  { name: "ROUND", signature: L("ROUND(number, digits)"), summary: L("round") },
  { name: "ABS", signature: "ABS(number)", summary: L("without sign") },
  { name: "MIN", signature: "MIN(a, b, …)", summary: L("smallest") },
  { name: "MAX", signature: "MAX(a, b, …)", summary: L("largest") },
  { name: "SUM", signature: "SUM(a, b, …)", summary: L("sum") },
  { name: "LEN", signature: L("LEN(text)"), summary: L("length") },
  { name: "UPPER", signature: L("UPPER(text)"), summary: L("upper case") },
  { name: "LOWER", signature: L("LOWER(text)"), summary: L("lower case") },
  { name: "TRIM", signature: L("TRIM(text)"), summary: L("without spaces at the ends") },
  { name: "LEFT", signature: L("LEFT(text, count)"), summary: L("first characters") },
  { name: "RIGHT", signature: L("RIGHT(text, count)"), summary: L("last characters") },
  { name: "MID", signature: L("MID(text, start, count)"), summary: L("characters from a position") },
  { name: "REPLACE", signature: L("REPLACE(text, find, with)"), summary: L("replace text") },
  { name: "CONTAINS", signature: L("CONTAINS(text, part)"), summary: L("whether the text contains the part") },
  { name: "CONCAT", signature: "CONCAT(a, b, …)", summary: L("join") },
  { name: "COALESCE", signature: "COALESCE(a, b, …)", summary: L("first value that is not empty") },
  { name: "ISEMPTY", signature: L("ISEMPTY(value)"), summary: L("whether the value is empty") },
  { name: "YEAR", signature: L("YEAR(date)"), summary: L("year") },
  { name: "MONTH", signature: L("MONTH(date)"), summary: L("month") },
  { name: "DAY", signature: L("DAY(date)"), summary: L("day of the month") },
  { name: "DAYS", signature: L("DAYS(end, start)"), summary: L("days between dates") },
];

export function describeFormula(problem: FormulaProblem): string {
  const { detail, position } = problem;
  switch (problem.kind) {
    case "unexpectedEnd":
      return L("The formula is incomplete.");
    case "unexpectedText":
      return detail === "" ? L("Unexpected text at position %lld.", position + 1) : L("Unexpected “%@” at position %lld.", detail, position + 1);
    case "unclosedText":
      return L("A quoted text is not closed.");
    case "unclosedColumn":
      return L("A column name is not closed with }.");
    case "unknownColumn":
      return L("There is no column “%@”.", detail);
    case "unknownFunction":
      return L("There is no function %@.", detail);
    case "wrongArgumentCount":
      return L("Wrong number of arguments for %@.", detail);
    case "notANumber":
      return L("“%@” is not a number", detail);
    case "notADate":
      return L("“%@” is not a date", detail);
    case "divisionByZero":
      return L("division by zero");
  }
}
