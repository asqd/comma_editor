import AppKit
import UniformTypeIdentifiers

// File ▸ Export As (JSON, Markdown, HTML, SQL, Excel) and Edit ▸ Copy As.
// Both take the visible rows, so sorting and filters shape what is exported.

extension GridViewController {
    /// Menu item tags.
    enum ExportKind: Int, CaseIterable {
        case json, markdown, html, sql, xlsx

        var format: ExportFormat {
            switch self {
            case .json: .json
            case .markdown: .markdown
            case .html: .html
            case .sql: .sql
            case .xlsx: .xlsx
            }
        }

        var contentType: UTType {
            switch self {
            case .json: .json
            case .markdown: UTType(filenameExtension: "md") ?? .plainText
            case .html: .html
            case .sql: UTType(filenameExtension: "sql") ?? .plainText
            case .xlsx: UTType(filenameExtension: "xlsx") ?? .data
            }
        }

        var fileExtension: String {
            switch self {
            case .json: "json"
            case .markdown: "md"
            case .html: "html"
            case .sql: "sql"
            case .xlsx: "xlsx"
            }
        }
    }

    /// Name for the HTML title, SQL table and sheet: the file name without its extension.
    private var exportName: String {
        let name = document.displayName ?? "data"
        return (name as NSString).deletingPathExtension
    }

    @objc func exportAs(_ sender: NSMenuItem) {
        guard let kind = ExportKind(rawValue: sender.tag), let window = view.window else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [kind.contentType]
        panel.nameFieldStringValue = "\(exportName).\(kind.fileExtension)"
        panel.canCreateDirectories = true
        let spec = core.viewSpec()
        if !spec.sort.isEmpty || !spec.filters.isEmpty {
            panel.message = L("Only the rows shown are exported, in the order shown.")
        }
        let core = core
        let name = exportName
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            // Large tables take seconds; build the file off the main thread.
            DispatchQueue.global(qos: .userInitiated).async {
                let result = Result { try core.export(format: kind.format, range: nil, name: name, fragment: false) }
                    .flatMap { data in Result { try data.write(to: url, options: .atomic) } }
                DispatchQueue.main.async {
                    if case .failure(let error) = result {
                        self?.presentExportError(error)
                    }
                }
            }
        }
    }

    /// Copies the selection, or the whole table when a single cell is selected.
    @objc func copyAs(_ sender: NSMenuItem) {
        guard let kind = ExportKind(rawValue: sender.tag), let selection else { return }
        let range = selection.isSingleCell ? nil : selection.range
        do {
            let data = try core.export(format: kind.format, range: range, name: exportName, fragment: true)
            let text = String(decoding: data, as: UTF8.self)
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            if kind == .html {
                pasteboard.setString(text, forType: .html)
            }
            pasteboard.setString(text, forType: .string)
        } catch {
            presentExportError(error)
        }
    }

    private func presentExportError(_ error: Error) {
        let error = CSVDocument.userFacingError(error)
        if let window = view.window {
            presentError(error, modalFor: window, delegate: nil, didPresent: nil, contextInfo: nil)
        }
    }
}
