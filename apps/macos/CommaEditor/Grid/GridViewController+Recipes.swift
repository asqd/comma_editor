import AppKit
import SwiftUI
import UniformTypeIdentifiers

// Recipes menu: recording changes as steps, applying saved recipes, managing them.

extension GridViewController {
    @objc func toggleRecipeRecording(_ sender: Any?) {
        guard document.recordedStepCount != nil else {
            document.startRecording()
            return
        }
        guard let recipe = document.stopRecording(), !recipe.steps().isEmpty else {
            inform(
                L("Nothing was recorded"),
                L("Recipes record changes to whole columns, columns themselves and the rows a filter selects. Edits of single cells or of chosen rows are not repeated on another file.")
            )
            return
        }
        let suggested = (document.displayName as NSString?)?.deletingPathExtension ?? ""
        ask(
            L("Save Recipe"),
            L("Steps recorded: %@. Name the recipe to apply it to the next file from the Recipes menu.", Self.format(recipe.steps().count)),
            text: suggested,
            label: L("Name:")
        ) { [weak self] name, _ in
            recipe.setName(name: name.trimmingCharacters(in: .whitespaces).isEmpty ? L("Untitled Recipe") : name)
            do {
                try RecipeStore.shared.save(recipe)
            } catch {
                self?.presentError(error)
            }
        }
    }

    /// Recipes ▸ a saved recipe: the item carries the recipe's file.
    @objc func applySavedRecipe(_ sender: NSMenuItem) {
        guard let url = sender.representedObject as? URL,
              let entry = RecipeStore.shared.all().first(where: { $0.url == url }) else { return }
        applyRecipe(entry.recipe)
    }

    func applyRecipe(_ recipe: CoreRecipe) {
        guard !document.isBusy else {
            NSSound.beep()
            return
        }
        document.applyRecipe(recipe)
    }

    @objc func manageRecipes(_ sender: Any?) {
        let model = RecipesModel()
        model.canApply = !document.isBusy
        let controller = NSHostingController(rootView: RecipesView(model: model))
        model.onClose = { [weak self, weak controller] in
            guard let controller else { return }
            self?.dismiss(controller)
        }
        model.onApply = { [weak self, weak controller] recipe in
            guard let self, let controller else { return }
            self.dismiss(controller)
            self.applyRecipe(recipe)
        }
        model.onExport = { [weak self, weak controller] recipe in
            guard let self, let window = controller?.view.window else { return }
            self.exportForPython(recipe, from: window)
        }
        presentAsSheet(controller)
    }

    /// Saves the recipe as a Python script with pandas that reads this file.
    private func exportForPython(_ recipe: CoreRecipe, from window: NSWindow) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.pythonScript]
        let base = recipe.name().isEmpty ? "recipe" : recipe.name()
        panel.nameFieldStringValue = base + ".py"
        let input = document.fileURL?.lastPathComponent ?? "input.csv"
        let output = ((input as NSString).deletingPathExtension) + "-clean." + ((input as NSString).pathExtension.isEmpty ? "csv" : (input as NSString).pathExtension)
        let script = recipe.toPandas(input: input, output: output, delimiter: info.delimiter)
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try script.write(to: url, atomically: true, encoding: .utf8)
            } catch {
                self?.presentError(error)
            }
        }
    }
}

/// The Recipes menu, rebuilt each time it opens with the saved recipes.
@MainActor
final class RecipesMenu: NSObject, NSMenuDelegate {
    static let shared = RecipesMenu()

    lazy var menu: NSMenu = {
        let menu = NSMenu(title: L("Recipes"))
        menu.delegate = self
        return menu
    }()

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.addItem(withTitle: L("Start Recording"), action: #selector(GridViewController.toggleRecipeRecording(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        let entries = RecipeStore.shared.all()
        if entries.isEmpty {
            let none = NSMenuItem(title: L("No Saved Recipes"), action: nil, keyEquivalent: "")
            none.isEnabled = false
            menu.addItem(none)
        }
        for entry in entries {
            let item = NSMenuItem(
                title: entry.name.isEmpty ? L("Untitled Recipe") : entry.name,
                action: #selector(GridViewController.applySavedRecipe(_:)),
                keyEquivalent: ""
            )
            item.representedObject = entry.url
            item.toolTip = entry.recipe.steps().enumerated().map { "\($0.offset + 1). " + RecipeText.describe($0.element) }.joined(separator: "\n")
            menu.addItem(item)
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: L("Manage Recipes…"), action: #selector(GridViewController.manageRecipes(_:)), keyEquivalent: "")
    }
}
