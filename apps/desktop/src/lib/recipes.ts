// Recipes as people read them (RecipeText in the macOS app): each step in words, with the
// filter that chose its rows.

import { L } from "./i18n";

export interface RecipeStep {
  kind: string;
  arguments: string[];
  filters: { column: string | null; op: string; value: string }[];
}

export interface Recipe {
  /** The recipe's file in the recipes' folder. */
  file: string;
  name: string;
  steps: RecipeStep[];
}

/** `stock = 0`, `city contains “Mos”`, `any column is empty`. */
function condition(filter: RecipeStep["filters"][number]): string {
  const column = filter.column ?? L("any column");
  const value = L("“%@”", filter.value);
  switch (filter.op) {
    case "contains":
      return L("%@ contains %@", column, value);
    case "not_contains":
      return L("%@ does not contain %@", column, value);
    case "equals":
      return `${column} = ${value}`;
    case "not_equals":
      return `${column} ≠ ${value}`;
    case "starts_with":
      return L("%@ starts with %@", column, value);
    case "ends_with":
      return L("%@ ends with %@", column, value);
    case "matches":
      return L("%@ matches %@", column, value);
    case "is_empty":
      return L("%@ is empty", column);
    case "is_not_empty":
      return L("%@ is not empty", column);
    case "greater_than":
      return `${column} > ${filter.value}`;
    case "less_than":
      return `${column} < ${filter.value}`;
    default:
      return column;
  }
}

export function describeStep(step: RecipeStep): string {
  const a = [...step.arguments, "", "", ""];
  const columns = (list: string) => (list === "" ? L("all columns") : list);
  const texts: Record<string, () => string> = {
    replace: () => L("Replace “%@” with “%@” in %@", a[0], a[1], columns(a[2])),
    trim: () => L("Trim spaces in %@", a[0]),
    collapse_spaces: () => L("Remove extra spaces in %@", a[0]),
    uppercase: () => L("UPPERCASE in %@", a[0]),
    lowercase: () => L("lowercase in %@", a[0]),
    capitalize: () => L("Capitalize words in %@", a[0]),
    fill_from_above: () => L("Fill empty cells of %@ from above", a[0]),
    fill_empty: () => L("Fill empty cells of %@ with “%@”", a[0], a[1]),
    fix_garbled: () => (a[0] === "" ? L("Fix garbled text in the whole file") : L("Fix garbled text in %@", a[0])),
    add_column: () => L("Add column “%@” = %@", a[0], a[1]),
    set_column: () => L("Write %@ into column %@", a[1], a[0]),
    split_column: () => L("Split column %@ at “%@”", a[0], a[1]),
    merge_columns: () => L("Merge columns %@", a[0]),
    remove_duplicates: () => L("Remove duplicate rows by %@", columns(a[0])),
    insert_column: () => (a[0] === "" ? L("Insert an empty column first") : L("Insert an empty column after %@", a[0])),
    delete_column: () => L("Delete column %@", a[0]),
    rename_column: () => L("Rename column %@ to “%@”", a[0], a[1]),
    move_column: () => (a[1] === "" ? L("Move column %@ to the start", a[0]) : L("Move column %@ after %@", a[0], a[1])),
    sort: () => L("Sort rows by %@", a[0]),
    delete_rows: () => L("Delete rows"),
    fix_rows: () => L("Repair broken rows"),
  };
  const text = texts[step.kind]?.() ?? step.kind;
  if (step.filters.length === 0) return text;
  const conditions = step.filters.map(condition).join(L(" and "));
  return step.kind === "delete_rows" ? text + L(" where %@", conditions) : `${text} ${L("(where %@)", conditions)}`;
}
