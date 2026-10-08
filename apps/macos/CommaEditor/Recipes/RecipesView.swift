import SwiftUI

/// Recipes ▸ Manage Recipes: saved recipes, their steps, and applying or exporting them.
@MainActor
final class RecipesModel: ObservableObject {
    @Published private(set) var entries: [RecipeStore.Entry] = []
    @Published var selection: URL? {
        didSet { loadSelected() }
    }
    @Published var name = ""
    @Published private(set) var steps: [RecipeStepInfo] = []
    @Published var selectedStep: Int?

    var onApply: ((CoreRecipe) -> Void)?
    var onExport: ((CoreRecipe) -> Void)?
    var onClose: (() -> Void)?
    /// Changes can be applied: a table is open and free.
    @Published var canApply = true

    init() {
        reload(selecting: nil)
    }

    var selected: RecipeStore.Entry? {
        entries.first { $0.url == selection }
    }

    func reload(selecting url: URL?) {
        entries = RecipeStore.shared.all()
        selection = url ?? entries.first?.url
    }

    private func loadSelected() {
        name = selected?.name ?? ""
        steps = selected?.recipe.steps() ?? []
        selectedStep = nil
    }

    func rename() {
        guard let entry = selected else { return }
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, trimmed != entry.name else { return }
        entry.recipe.setName(name: trimmed)
        _ = try? RecipeStore.shared.save(entry.recipe, to: entry.url)
        reload(selecting: entry.url)
    }

    func removeStep() {
        guard let entry = selected, let index = selectedStep else { return }
        entry.recipe.removeStep(index: UInt64(index))
        _ = try? RecipeStore.shared.save(entry.recipe, to: entry.url)
        steps = entry.recipe.steps()
        selectedStep = nil
    }

    func delete() {
        guard let entry = selected else { return }
        RecipeStore.shared.delete(entry)
        reload(selecting: nil)
    }
}

struct RecipesView: View {
    @ObservedObject var model: RecipesModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("Recipes")).font(.headline)
            if model.entries.isEmpty {
                Text(L("No recipes yet. Choose Recipes ▸ Start Recording, clean up a table as usual and stop recording: the steps can then be repeated on the next file."))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else {
                HStack(alignment: .top, spacing: 12) {
                    List(model.entries, id: \.url, selection: $model.selection) { entry in
                        Text(entry.name.isEmpty ? L("Untitled Recipe") : entry.name).tag(entry.url)
                    }
                    .frame(width: 190)
                    VStack(alignment: .leading, spacing: 8) {
                        TextField(L("Name"), text: $model.name)
                            .onSubmit { model.rename() }
                        List(Array(model.steps.enumerated()), id: \.offset, selection: $model.selectedStep) { index, step in
                            Text("\(index + 1). " + RecipeText.describe(step))
                                .lineLimit(2)
                                .tag(index)
                        }
                        HStack {
                            Button(L("Delete Step")) { model.removeStep() }
                                .disabled(model.selectedStep == nil)
                            Spacer()
                            Button(L("Delete Recipe")) { model.delete() }
                                .disabled(model.selected == nil)
                        }
                    }
                }
            }
            HStack {
                Button(L("Export for Python…")) {
                    model.rename()
                    if let recipe = model.selected?.recipe { model.onExport?(recipe) }
                }
                .disabled(model.selected == nil)
                Spacer()
                Button(L("Done")) {
                    model.rename()
                    model.onClose?()
                }
                .keyboardShortcut(.cancelAction)
                Button(L("Apply to This Table")) {
                    model.rename()
                    if let recipe = model.selected?.recipe { model.onApply?(recipe) }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(model.selected == nil || model.steps.isEmpty || !model.canApply)
            }
        }
        .padding(20)
        .frame(width: 680, height: 420)
    }
}
