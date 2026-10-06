import AppKit

/// A CSV file open in the app. The data and the edit history live in the Rust core;
/// each core edit is mirrored by one step in the document's undo manager.
final class CSVDocument: NSDocument {
    enum Change {
        /// One cell changed; row is a core row (the header row counts).
        case cell(row: Int, column: Int)
        /// Rows, columns or the header setting changed.
        case structure
    }

    static let didChangeNotification = Notification.Name("CSVDocumentDidChange")
    static let changeKey = "change"

    private(set) var core: CoreDocument

    override init() {
        core = CoreDocument.newEmpty()
        super.init()
    }

    override class var autosavesInPlace: Bool { true }

    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { true }

    override func makeWindowControllers() {
        addWindowController(DocumentWindowController())
    }

    // MARK: Reading and writing

    override func read(from url: URL, ofType typeName: String) throws {
        do {
            core = try CoreDocument.open(path: url.path)
        } catch {
            throw Self.userFacingError(error)
        }
    }

    override func revert(toContentsOf url: URL, ofType typeName: String) throws {
        try super.revert(toContentsOf: url, ofType: typeName)
        undoManager?.removeAllActions()
        post(.structure)
    }

    /// Unchanged records are copied byte for byte, so saving never reformats the file.
    override func write(to url: URL, ofType typeName: String) throws {
        do {
            try core.save(path: url.path)
        } catch {
            throw Self.userFacingError(error)
        }
    }

    // MARK: Editing

    @discardableResult
    func setCell(row: Int, column: Int, value: String) -> Bool {
        perform(L("Cell Edit"), .cell(row: row, column: column)) {
            try $0.setCell(row: UInt64(row), column: UInt64(column), value: value)
        }
    }

    @discardableResult
    func renameColumn(_ column: Int, to name: String) -> Bool {
        perform(L("Column Rename"), .cell(row: 0, column: column)) {
            try $0.setCell(row: 0, column: UInt64(column), value: name)
        }
    }

    @discardableResult
    func insertRows(at row: Int, count: Int) -> Bool {
        perform(L("Row Insertion"), .structure) {
            try $0.insertRows(at: UInt64(row), count: UInt64(count))
            return true
        }
    }

    @discardableResult
    func deleteRows(_ rows: [Int]) -> Bool {
        perform(L("Row Deletion"), .structure) {
            try $0.deleteRows(rows: rows.map { UInt64($0) })
            return true
        }
    }

    @discardableResult
    func insertColumn(at column: Int) -> Bool {
        perform(L("Column Insertion"), .structure) {
            try $0.insertColumn(at: UInt64(column))
            return true
        }
    }

    @discardableResult
    func deleteColumn(at column: Int) -> Bool {
        perform(L("Column Deletion"), .structure) {
            try $0.deleteColumn(at: UInt64(column))
            return true
        }
    }

    /// Display setting only: the file does not change.
    func setHasHeader(_ hasHeader: Bool) {
        core.setHasHeader(hasHeader: hasHeader)
        post(.structure)
    }

    /// Runs one core edit. If it changed something, registers an undo step and notifies the UI.
    private func perform(_ actionName: String, _ change: Change, _ edit: (CoreDocument) throws -> Bool) -> Bool {
        do {
            guard try edit(core) else { return false }
        } catch {
            presentEditError(error)
            return false
        }
        registerUndo()
        undoManager?.setActionName(actionName)
        post(change)
        return true
    }

    // The core keeps the history; the undo manager only drives it, so NSDocument tracks
    // unsaved changes and the Edit menu shows the right titles.
    private func registerUndo() {
        undoManager?.registerUndo(withTarget: self) { document in
            _ = document.core.undo()
            document.registerRedo()
            document.post(.structure)
        }
    }

    private func registerRedo() {
        undoManager?.registerUndo(withTarget: self) { document in
            _ = document.core.redo()
            document.registerUndo()
            document.post(.structure)
        }
    }

    private func post(_ change: Change) {
        NotificationCenter.default.post(
            name: Self.didChangeNotification,
            object: self,
            userInfo: [Self.changeKey: change]
        )
    }

    // MARK: Errors

    private func presentEditError(_ error: Error) {
        let error = Self.userFacingError(error)
        if let window = windowForSheet {
            presentError(error, modalFor: window, delegate: nil, didPresent: nil, contextInfo: nil)
        } else {
            presentError(error)
        }
    }

    static func userFacingError(_ error: Error) -> NSError {
        guard let error = error as? CoreError else { return error as NSError }
        var userInfo: [String: Any] = [:]
        switch error {
        case .Io(let message), .OutOfRange(let message):
            userInfo[NSLocalizedDescriptionKey] = message
        case .Unmappable(let row, let column, let encoding):
            userInfo[NSLocalizedDescriptionKey] = L(
                "The value in row %lld, column %lld cannot be saved in %@.",
                Int64(row) + 1, Int64(column) + 1, encoding
            )
            userInfo[NSLocalizedRecoverySuggestionErrorKey] = L(
                "Replace the characters that this encoding cannot represent."
            )
        case .LastColumn:
            userInfo[NSLocalizedDescriptionKey] = L("The last column cannot be deleted.")
        }
        return NSError(domain: "app.commaeditor", code: 1, userInfo: userInfo)
    }
}
