import Foundation
import QuickLookUI
import UniformTypeIdentifiers

/// Quick Look preview of CSV and TSV files in Finder: the first rows as a table, read with the
/// same encoding and delimiter detection as the app.
final class PreviewProvider: QLPreviewProvider, QLPreviewingController {
    private static let rows: UInt64 = 500

    func providePreview(for request: QLFilePreviewRequest) async throws -> QLPreviewReply {
        let url = request.fileURL
        let accessing = url.startAccessingSecurityScopedResource()
        defer {
            if accessing {
                url.stopAccessingSecurityScopedResource()
            }
        }
        let preview = try previewFile(path: url.path, encoding: nil, delimiter: nil, rows: Self.rows)
        let html = PreviewPage.html(for: preview, limit: Int(Self.rows))
        return QLPreviewReply(dataOfContentType: .html, contentSize: CGSize(width: 960, height: 640)) { _ in
            Data(html.utf8)
        }
    }
}
