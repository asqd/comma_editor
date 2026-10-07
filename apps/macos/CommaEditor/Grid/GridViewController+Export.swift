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
            self?.exportsRunning += 1
            self?.updateStatus()
            // Large tables take seconds: the file is built off the main thread, written to a
            // temporary folder and then put in place, so a failed export leaves no half file.
            DispatchQueue.global(qos: .userInitiated).async {
                let result = Result { try Self.export(core, kind: kind, name: name, to: url) }
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.exportsRunning -= 1
                    self.updateStatus()
                    if case .failure(let error) = result {
                        self.presentExportError(error)
                    }
                }
            }
        }
    }

    #if DEBUG
    /// Exports to the app's temporary folder without the save panel, and logs the result.
    func debugExport(_ kind: ExportKind) {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("debug-export.\(kind.fileExtension)")
        let core = core
        let name = exportName
        exportsRunning += 1
        updateStatus()
        let started = Date()
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try Self.export(core, kind: kind, name: name, to: url) }
            let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? -1
            NSLog("COMMA export \(kind) \(result) in \(Date().timeIntervalSince(started)) s, \(size) bytes")
            DispatchQueue.main.async { [weak self] in
                self?.exportsRunning -= 1
                self?.updateStatus()
            }
        }
    }
    #endif

    private nonisolated static func export(_ core: CoreDocument, kind: ExportKind, name: String, to url: URL) throws {
        let files = FileManager.default
        let folder = try files.url(for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: url, create: true)
        defer { try? files.removeItem(at: folder) }
        let temporary = folder.appendingPathComponent(url.lastPathComponent)
        try core.exportToFile(path: temporary.path, format: kind.format, name: name)
        if files.fileExists(atPath: url.path) {
            _ = try files.replaceItemAt(url, withItemAt: temporary)
        } else {
            try files.moveItem(at: temporary, to: url)
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
