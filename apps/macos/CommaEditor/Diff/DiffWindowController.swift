import AppKit

/// File ▸ Compare with File: the rows of a table and of another file matched by a key column,
/// the rows found in only one of them and the values that differ. The other file is read and
/// compared off the main thread; the result is read a page of rows at a time.
final class DiffWindowController: NSWindowController, NSWindowDelegate {
    /// Open comparison windows, which nothing else keeps.
    private static var shown: [DiffWindowController] = []

    /// Shows a comparison of the grid's table with the file at `url`.
    static func compare(_ grid: GridViewController, with url: URL) {
        let controller = DiffWindowController(grid: grid, url: url)
        shown.append(controller)
        controller.showWindow(nil)
        controller.readOtherFile()
    }

    private weak var grid: GridViewController?
    private let left: CoreDocument
    private let leftName: String
    private let url: URL
    private var right: CoreDocument?
    private var job: DiffJob?
    private var result: DiffResult?
    private var progressTimer: Timer?
    private var shownColumns: [DiffColumnInfo] = []
    private var rowCount = 0
    /// Header rows of each table, so row numbers read as in the tables.
    private var headerRows = (left: 0, right: 0)
    private var pages: [Int: [DiffRow]] = [:]
    private static let pageSize = 200

    private let keyPopup = NSPopUpButton()
    private let caseCheck = NSButton(checkboxWithTitle: L("Ignore case"), target: nil, action: nil)
    private let trimCheck = NSButton(checkboxWithTitle: L("Ignore surrounding spaces"), target: nil, action: nil)
    private let filter = NSSegmentedControl()
    private let openButton = NSButton(title: L("Open as Table"), target: nil, action: nil)
    private let tableView = NSTableView()
    private let statusLabel = NSTextField(labelWithString: "")
    private let spinner = NSProgressIndicator()
    private let cancelButton = NSButton(title: L("Cancel"), target: nil, action: nil)

    /// What the filter segments show, in order.
    private var filters: [[DiffStatus]] {
        [[.onlyLeft, .onlyRight, .changed], [.onlyLeft], [.onlyRight], [.changed], [.same]]
    }

    private init(grid: GridViewController, url: URL) {
        self.grid = grid
        left = grid.document.core
        leftName = grid.document.displayName ?? L("Untitled")
        self.url = url
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: true
        )
        window.minSize = NSSize(width: 560, height: 300)
        window.title = L("%@ and %@", leftName, url.lastPathComponent)
        window.subtitle = L("Comparison")
        super.init(window: window)
        window.delegate = self
        window.contentView = makeContent()
        if let parent = grid.view.window {
            window.setFrameTopLeftPoint(NSPoint(x: parent.frame.minX + 40, y: parent.frame.maxY - 40))
        } else {
            window.center()
        }
        setControlsEnabled(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func windowWillClose(_ notification: Notification) {
        job?.cancel()
        progressTimer?.invalidate()
        Self.shown.removeAll { $0 === self }
    }

    // MARK: Layout

    private func makeContent() -> NSView {
        keyPopup.target = self
        keyPopup.action = #selector(optionsChanged(_:))
        for check in [caseCheck, trimCheck] {
            check.target = self
            check.action = #selector(optionsChanged(_:))
        }
        openButton.target = self
        openButton.action = #selector(openAsTable(_:))
        openButton.bezelStyle = .push
        let options = NSStackView(views: [
            NSTextField(labelWithString: L("Match rows by:")), keyPopup, caseCheck, trimCheck, NSView(), openButton,
        ])
        options.spacing = 10

        filter.segmentStyle = .rounded
        filter.trackingMode = .selectOne
        filter.segmentCount = filters.count
        filter.selectedSegment = 0
        filter.target = self
        filter.action = #selector(filterChanged(_:))
        updateFilterLabels(nil)

        tableView.style = .fullWidth
        tableView.usesAlternatingRowBackgroundColors = false
        tableView.gridStyleMask = [.solidVerticalGridLineMask]
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.allowsColumnReordering = false
        tableView.rowHeight = 22
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(revealRow(_:))
        let scroll = NSScrollView()
        scroll.documentView = tableView
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.borderType = .bezelBorder

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingTail
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        cancelButton.bezelStyle = .inline
        cancelButton.controlSize = .small
        cancelButton.target = self
        cancelButton.action = #selector(cancel(_:))
        cancelButton.isHidden = true
        let status = NSStackView(views: [spinner, statusLabel, cancelButton, NSView()])
        status.spacing = 6

        let stack = NSStackView(views: [options, filter, scroll, status])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 16, bottom: 10, right: 16)
        for view in [options, scroll, status] {
            view.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -32).isActive = true
        }
        scroll.setContentHuggingPriority(.defaultLow, for: .vertical)
        return stack
    }

    private func setControlsEnabled(_ enabled: Bool) {
        for control in [keyPopup, caseCheck, trimCheck, filter, openButton] as [NSControl] {
            control.isEnabled = enabled
        }
        openButton.isEnabled = enabled && result != nil && rowCount > 0
    }

    // MARK: Reading the other file

    private func readOtherFile() {
        showProgress(L("Reading %@…", url.lastPathComponent), cancellable: false)
        let url = url
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let opened = Result { () throws -> CoreDocument in
                let document = try CoreDocument.openWith(path: url.path, encoding: nil, delimiter: nil)
                while document.isIndexing() {
                    _ = document.continueIndexing(bytes: 64 << 20)
                }
                return document
            }
            DispatchQueue.main.async {
                guard let self else { return }
                switch opened {
                case .success(let document):
                    self.right = document
                    self.headerRows = (self.left.info().hasHeader ? 1 : 0, document.info().hasHeader ? 1 : 0)
                    self.fillKeyPopup(document)
                    self.compare()
                case .failure(let error):
                    self.showFinished(CSVDocument.userFacingError(error).localizedDescription)
                }
            }
        }
    }

    /// Columns both tables have, the suggested key selected; "Row Number" compares by position.
    private func fillKeyPopup(_ right: CoreDocument) {
        keyPopup.removeAllItems()
        keyPopup.addItem(withTitle: L("Row Number"))
        keyPopup.lastItem?.tag = -1
        // Not 0, the tag of the first column.
        let separator = NSMenuItem.separator()
        separator.tag = Int.min
        keyPopup.menu?.addItem(separator)
        for (index, column) in left.diffColumns(other: right).enumerated() {
            guard let leftColumn = column.left, column.right != nil else { continue }
            keyPopup.addItem(withTitle: columnName(column, index: index))
            keyPopup.lastItem?.tag = Int(leftColumn)
        }
        let suggested = left.suggestDiffKey(other: right).first.map { Int($0) } ?? -1
        keyPopup.selectItem(withTag: suggested)
    }

    private func columnName(_ column: DiffColumnInfo, index: Int) -> String {
        if let name = column.name, !name.isEmpty {
            return name
        }
        return GridViewController.columnLetter(Int(column.left ?? column.right ?? UInt64(index)))
    }

    // MARK: Comparing

    @objc private func optionsChanged(_ sender: Any?) {
        compare()
    }

    private func compare() {
        guard let right else { return }
        job?.cancel()
        let key = keyPopup.selectedTag()
        let options = DiffOptions(
            key: key < 0 ? [] : [UInt64(key)],
            ignoreCase: caseCheck.state == .on,
            trim: trimCheck.state == .on
        )
        let job: DiffJob
        do {
            job = try left.beginDiff(other: right, options: options)
        } catch {
            showFinished(CSVDocument.userFacingError(error).localizedDescription)
            return
        }
        self.job = job
        setControlsEnabled(false)
        showProgress(L("Comparing…"), cancellable: true)
        progressTimer?.invalidate()
        let timer = Timer(timeInterval: 0.1, repeats: true) { [weak self, weak job] _ in
            guard let self, let job, self.job === job else { return }
            let percent = job.fraction().formatted(.percent.precision(.fractionLength(0)))
            self.statusLabel.stringValue = L("Comparing…") + " " + percent
        }
        RunLoop.main.add(timer, forMode: .common)
        progressTimer = timer
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let outcome = Result { try job.run() }
            DispatchQueue.main.async {
                guard let self, self.job === job else { return }
                self.job = nil
                self.progressTimer?.invalidate()
                switch outcome {
                case .success(let result?):
                    self.show(result)
                case .success(nil):
                    self.showFinished(L("The comparison was cancelled."))
                    self.setControlsEnabled(self.right != nil)
                case .failure(let error):
                    self.showFinished(CSVDocument.userFacingError(error).localizedDescription)
                    self.setControlsEnabled(true)
                }
            }
        }
    }

    @objc private func cancel(_ sender: Any?) {
        job?.cancel()
    }

    private func show(_ result: DiffResult) {
        self.result = result
        shownColumns = result.columns()
        rebuildColumns()
        updateFilterLabels(result.summary())
        reload()
        setControlsEnabled(true)
        showFinished(summaryText(result.summary()))
    }

    private func summaryText(_ summary: DiffSummary) -> String {
        var parts: [String] = []
        let differences = summary.onlyLeft + summary.onlyRight + summary.changed
        parts.append(differences == 0
            ? L("The tables match.")
            : L("Differences: %@.", GridViewController.format(Int(differences))))
        if summary.repeatedLeft + summary.repeatedRight > 0 {
            parts.append(L("Rows with a repeated key: %@ in %@, %@ in %@; they are paired in order.",
                           GridViewController.format(Int(summary.repeatedLeft)), leftName,
                           GridViewController.format(Int(summary.repeatedRight)), url.lastPathComponent))
        }
        let oneSided = shownColumns.filter { $0.left == nil || $0.right == nil }.count
        if oneSided > 0 {
            parts.append(L("Columns only in one file are shown but not compared: %@.", GridViewController.format(oneSided)))
        }
        parts.append(L("Double-click a row to show it in the table."))
        return parts.joined(separator: " ")
    }

    private func updateFilterLabels(_ summary: DiffSummary?) {
        let counts: [UInt64]? = summary.map { summary in
            [summary.onlyLeft + summary.onlyRight + summary.changed, summary.onlyLeft, summary.onlyRight, summary.changed, summary.same]
        }
        let titles = [
            L("All Differences"), L("Only in %@", leftName), L("Only in %@", url.lastPathComponent), L("Changed"), L("Same"),
        ]
        for (index, title) in titles.enumerated() {
            let count = counts.map { " (" + GridViewController.format(Int($0[index])) + ")" } ?? ""
            filter.setLabel(title + count, forSegment: index)
            filter.setWidth(0, forSegment: index)
        }
    }

    @objc private func filterChanged(_ sender: Any?) {
        reload()
    }

    private var statuses: [DiffStatus] {
        filters[max(filter.selectedSegment, 0)]
    }

    private func reload() {
        pages.removeAll()
        rowCount = result.map { Int($0.count(statuses: statuses)) } ?? 0
        tableView.reloadData()
        tableView.scrollRowToVisible(0)
        openButton.isEnabled = result != nil && rowCount > 0
    }

    // MARK: Status line

    private func showProgress(_ text: String, cancellable: Bool) {
        statusLabel.stringValue = text
        spinner.startAnimation(nil)
        cancelButton.isHidden = !cancellable
    }

    private func showFinished(_ text: String) {
        statusLabel.stringValue = text
        spinner.stopAnimation(nil)
        cancelButton.isHidden = true
    }

    // MARK: Table

    private static let statusColumn = NSUserInterfaceItemIdentifier("status")
    private static let rowColumn = NSUserInterfaceItemIdentifier("row")

    private func rebuildColumns() {
        for column in tableView.tableColumns {
            tableView.removeTableColumn(column)
        }
        let status = NSTableColumn(identifier: Self.statusColumn)
        status.title = ""
        status.width = 22
        status.resizingMask = []
        tableView.addTableColumn(status)
        let rows = NSTableColumn(identifier: Self.rowColumn)
        rows.title = L("Rows")
        rows.width = 110
        tableView.addTableColumn(rows)
        for (index, info) in shownColumns.enumerated() {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(index)"))
            var title = columnName(info, index: index)
            if info.left == nil {
                title += " " + L("(only in %@)", url.lastPathComponent)
            } else if info.right == nil {
                title += " " + L("(only in %@)", leftName)
            }
            column.title = title
            column.headerToolTip = title
            column.width = 150
            column.minWidth = 40
            tableView.addTableColumn(column)
        }
    }

    private func row(_ index: Int) -> DiffRow? {
        guard let result else { return nil }
        let page = index / Self.pageSize
        if pages[page] == nil {
            if pages.count > 50 {
                pages.removeAll()
            }
            pages[page] = result.rows(statuses: statuses, start: UInt64(page * Self.pageSize), count: UInt64(Self.pageSize))
        }
        let rows = pages[page] ?? []
        let offset = index % Self.pageSize
        return offset < rows.count ? rows[offset] : nil
    }

    /// Shows the left row of the double-clicked comparison row in the document's table.
    @objc private func revealRow(_ sender: Any?) {
        let index = tableView.clickedRow
        guard index >= 0, let fileRow = row(index)?.leftRow, let grid else {
            NSSound.beep()
            return
        }
        grid.view.window?.makeKeyAndOrderFront(nil)
        grid.reveal(fileRow: Int(fileRow))
    }

    // MARK: Open as Table

    @objc private func openAsTable(_ sender: Any?) {
        guard let result else { return }
        let names = [
            L("Only in %@", leftName), L("Only in %@", url.lastPathComponent), L("Changed Row"), L("Same Row"),
        ]
        let data = result.toCsv(statuses: statuses, names: names, statusTitle: L("Comparison"))
        let document = CSVDocument(core: CoreDocument.fromBytes(data: data))
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        document.showWindows()
    }
}

#if DEBUG
extension DiffWindowController {
    /// Debug scripts: `filter:<segment>`, `key:<column or -1>`, `case`, `open`.
    static func debugStep(_ step: String) {
        guard let controller = shown.last else { return }
        let (name, value) = (step.split(separator: ":").first.map(String.init) ?? "", step.split(separator: ":").last.map { Int($0) ?? 0 } ?? 0)
        switch name {
        case "filter":
            controller.filter.selectedSegment = value
            controller.filterChanged(nil)
        case "key":
            controller.keyPopup.selectItem(withTag: value)
            controller.optionsChanged(nil)
        case "case":
            controller.caseCheck.state = controller.caseCheck.state == .on ? .off : .on
            controller.optionsChanged(nil)
        case "open":
            controller.openAsTable(nil)
        default:
            NSLog("COMMA diff: unknown step \(step)")
        }
    }
}
#endif

extension DiffWindowController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rowCount
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row index: Int) -> NSView? {
        guard let identifier = tableColumn?.identifier, let row = row(index) else { return nil }
        let label = tableView.makeView(withIdentifier: identifier, owner: self) as? NSTextField
            ?? {
                let label = NSTextField(labelWithString: "")
                label.identifier = identifier
                label.lineBreakMode = .byTruncatingTail
                label.cell?.truncatesLastVisibleLine = true
                return label
            }()
        label.toolTip = nil
        switch identifier {
        case Self.statusColumn:
            let (mark, color) = Self.mark(row.status)
            label.attributedStringValue = NSAttributedString(string: mark, attributes: [
                .foregroundColor: color, .font: NSFont.boldSystemFont(ofSize: NSFont.systemFontSize),
            ])
            label.alignment = .center
        case Self.rowColumn:
            let number = { (row: UInt64?, offset: Int) in row.map { GridViewController.format(Int($0) - offset + 1) } ?? "—" }
            label.stringValue = number(row.leftRow, headerRows.left) + " · " + number(row.rightRow, headerRows.right)
            label.textColor = .secondaryLabelColor
            label.alignment = .right
        default:
            let column = Int(identifier.rawValue.dropFirst()) ?? 0
            guard column < row.cells.count else { return label }
            label.alignment = .natural
            label.attributedStringValue = Self.text(row.cells[column], status: row.status)
            if row.cells[column].changed {
                label.toolTip = (row.cells[column].left ?? "") + " → " + (row.cells[column].right ?? "")
            }
        }
        return label
    }

    func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow index: Int) {
        guard let row = row(index) else { return }
        switch row.status {
        case .onlyLeft: rowView.backgroundColor = NSColor.systemRed.withAlphaComponent(0.08)
        case .onlyRight: rowView.backgroundColor = NSColor.systemGreen.withAlphaComponent(0.08)
        case .changed, .same: rowView.backgroundColor = .clear
        }
    }

    private static func mark(_ status: DiffStatus) -> (String, NSColor) {
        switch status {
        case .onlyLeft: ("−", .systemRed)
        case .onlyRight: ("+", .systemGreen)
        case .changed: ("≠", .systemOrange)
        case .same: ("=", .tertiaryLabelColor)
        }
    }

    /// A value, or for a changed one the old value struck through and the new one.
    private static func text(_ cell: DiffCellValue, status: DiffStatus) -> NSAttributedString {
        let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        let plain = { (value: String, color: NSColor) in
            NSAttributedString(string: GridViewController.singleLine(value), attributes: [.font: font, .foregroundColor: color])
        }
        guard cell.changed, let old = cell.left, let new = cell.right else {
            let value = cell.right ?? cell.left ?? ""
            return plain(value, status == .same ? .secondaryLabelColor : .labelColor)
        }
        let text = NSMutableAttributedString(string: GridViewController.singleLine(old), attributes: [
            .font: font,
            .foregroundColor: NSColor.systemRed,
            .strikethroughStyle: NSUnderlineStyle.single.rawValue,
        ])
        text.append(plain("  ", .labelColor))
        text.append(NSAttributedString(string: GridViewController.singleLine(new), attributes: [
            .font: NSFont.boldSystemFont(ofSize: NSFont.systemFontSize),
            .foregroundColor: NSColor.labelColor,
        ]))
        return text
    }
}
