import Foundation

/// Saved recipes: one JSON file each in the app's Application Support folder.
@MainActor
final class RecipeStore {
    static let shared = RecipeStore()

    struct Entry {
        let url: URL
        let recipe: CoreRecipe
        var name: String { recipe.name() }
    }

    private var directory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("CommaEditor/Recipes", isDirectory: true)
    }

    /// Recipes by name; files that do not read as recipes are left out.
    func all() -> [Entry] {
        let files = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
        return files
            .filter { $0.pathExtension == "json" }
            .compactMap { url in
                guard let text = try? String(contentsOf: url, encoding: .utf8),
                      let recipe = try? CoreRecipe.fromJson(json: text) else { return nil }
                return Entry(url: url, recipe: recipe)
            }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// Writes a recipe to its file, or to a new one; returns the file.
    @discardableResult
    func save(_ recipe: CoreRecipe, to url: URL? = nil) throws -> URL {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = url ?? directory.appendingPathComponent(UUID().uuidString).appendingPathExtension("json")
        try recipe.toJson().write(to: url, atomically: true, encoding: .utf8)
        return url
    }

    func delete(_ entry: Entry) {
        try? FileManager.default.removeItem(at: entry.url)
    }
}

/// Recipe steps in words.
enum RecipeText {
    static func describe(_ step: RecipeStepInfo) -> String {
        let a = step.arguments + Array(repeating: "", count: max(0, 3 - step.arguments.count))
        let columns = { (list: String) in list.isEmpty ? L("all columns") : list }
        let text: String = switch step.kind {
        case "replace": L("Replace “%@” with “%@” in %@", a[0], a[1], columns(a[2]))
        case "trim": L("Trim spaces in %@", a[0])
        case "collapse_spaces": L("Remove extra spaces in %@", a[0])
        case "uppercase": L("UPPERCASE in %@", a[0])
        case "lowercase": L("lowercase in %@", a[0])
        case "capitalize": L("Capitalize words in %@", a[0])
        case "fill_from_above": L("Fill empty cells of %@ from above", a[0])
        case "fill_empty": L("Fill empty cells of %@ with “%@”", a[0], a[1])
        case "fix_garbled": a[0].isEmpty ? L("Fix garbled text in the whole file") : L("Fix garbled text in %@", a[0])
        case "add_column": L("Add column “%@” = %@", a[0], a[1])
        case "set_column": L("Write %@ into column %@", a[1], a[0])
        case "split_column": L("Split column %@ at “%@”", a[0], a[1])
        case "merge_columns": L("Merge columns %@", a[0])
        case "remove_duplicates": L("Remove duplicate rows by %@", columns(a[0]))
        case "insert_column": a[0].isEmpty ? L("Insert an empty column first") : L("Insert an empty column after %@", a[0])
        case "delete_column": L("Delete column %@", a[0])
        case "rename_column": L("Rename column %@ to “%@”", a[0], a[1])
        case "move_column": a[1].isEmpty ? L("Move column %@ to the start", a[0]) : L("Move column %@ after %@", a[0], a[1])
        case "sort": L("Sort rows by %@", a[0])
        case "delete_rows": L("Delete rows")
        case "fix_rows": L("Repair broken rows")
        default: step.kind
        }
        guard !step.filters.isEmpty else { return text }
        let conditions = step.filters.map(condition).joined(separator: L(" and "))
        return step.kind == "delete_rows" ? text + L(" where %@", conditions) : text + " " + L("(where %@)", conditions)
    }

    /// `stock = 0`, `city contains “Mos”`, `any column is empty`.
    private static func condition(_ filter: RecipeFilterInfo) -> String {
        let column = filter.column ?? L("any column")
        let value = L("“%@”", filter.value)
        return switch filter.op {
        case "contains": L("%@ contains %@", column, value)
        case "not_contains": L("%@ does not contain %@", column, value)
        case "equals": "\(column) = \(value)"
        case "not_equals": "\(column) ≠ \(value)"
        case "starts_with": L("%@ starts with %@", column, value)
        case "ends_with": L("%@ ends with %@", column, value)
        case "matches": L("%@ matches %@", column, value)
        case "is_empty": L("%@ is empty", column)
        case "is_not_empty": L("%@ is not empty", column)
        case "greater_than": "\(column) > \(filter.value)"
        case "less_than": "\(column) < \(filter.value)"
        default: column
        }
    }
}
