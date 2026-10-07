import AppKit

// Large files: rows that arrive while the file is read, the status of reading and of
// background sorting, and the commands that wait until the file is read.

extension GridViewController {
    func configureBusyViews() -> NSView {
        busyIndicator.style = .spinning
        busyIndicator.controlSize = .small
        busyIndicator.isDisplayedWhenStopped = false
        busyLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        busyLabel.textColor = .secondaryLabelColor
        busyCancel.title = L("Cancel")
        busyCancel.bezelStyle = .inline
        busyCancel.controlSize = .small
        busyCancel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        busyCancel.target = self
        busyCancel.action = #selector(cancelViewUpdate(_:))
        let stack = NSStackView(views: [busyIndicator, busyLabel, busyCancel])
        stack.spacing = 6
        stack.isHidden = true
        return stack
    }

    /// Reading a large file, or sorting and filtering in the background.
    func updateBusyState() {
        let stack = busyLabel.superview
        if let fraction = document.indexingFraction {
            busyLabel.stringValue = L("Reading the file: %@", fraction.formatted(.percent.precision(.fractionLength(0))))
            busyCancel.isHidden = true
        } else if document.isUpdatingView {
            busyLabel.stringValue = L("Sorting and filtering…")
            busyCancel.isHidden = false
        } else if exportsRunning > 0 {
            busyLabel.stringValue = L("Exporting…")
            busyCancel.isHidden = true
        } else {
            busyIndicator.stopAnimation(nil)
            stack?.isHidden = true
            return
        }
        stack?.isHidden = false
        busyIndicator.startAnimation(nil)
    }

    @objc func cancelViewUpdate(_ sender: Any?) {
        document.cancelViewUpdate()
    }

    /// More rows of a large file were read: the table grows without reloading what it shows.
    func rowsAdded(since previous: DocumentInfo) {
        if info.columnCount != previous.columnCount {
            rebuildColumns()
            reloadTables()
        } else {
            for table in tables {
                table.noteNumberOfRowsChanged()
            }
            if String(info.rowCount).count != String(previous.rowCount).count {
                updateRowNumberWidth()
            }
        }
        updateStatus()
    }

    /// Commands that change the file, which wait until a large file is read whole.
    static let changingActions: Set<Selector> = [
        #selector(deleteRows(_:)), #selector(delete(_:)), #selector(cut(_:)), #selector(paste(_:)),
        #selector(fillDown(_:)), #selector(insertRowAbove(_:)), #selector(insertRowBelow(_:)),
        #selector(insertColumnLeft(_:)), #selector(insertColumnRight(_:)), #selector(deleteColumn(_:)),
        #selector(renameColumn(_:)), #selector(editCell(_:)), #selector(editCellInWindow(_:)),
        #selector(trimSpaces(_:)), #selector(collapseSpaces(_:)), #selector(makeUppercase(_:)),
        #selector(makeLowercase(_:)), #selector(capitalizeWords(_:)), #selector(fillEmptyFromAbove(_:)),
        #selector(fillEmptyWithValue(_:)), #selector(splitColumn(_:)), #selector(mergeColumns(_:)),
        #selector(removeDuplicateRows(_:)), #selector(computedColumn(_:)), #selector(fixGarbledText(_:)),
        #selector(moveRowsUp(_:)), #selector(moveRowsDown(_:)), #selector(moveColumnLeft(_:)),
        #selector(moveColumnRight(_:)), #selector(applySortToFile(_:)), #selector(toggleHeaderRow(_:)),
        #selector(sortAscending(_:)), #selector(sortDescending(_:)), #selector(clearSort(_:)),
        #selector(toggleFilterBar(_:)), #selector(clearFilter(_:)), #selector(showFindAndReplaceBar(_:)),
    ]
}
