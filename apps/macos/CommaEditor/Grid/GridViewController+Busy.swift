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
        busyCancel.action = #selector(cancelBackgroundWork(_:))
        let stack = NSStackView(views: [busyIndicator, busyLabel, busyCancel])
        stack.spacing = 6
        stack.isHidden = true
        return stack
    }

    /// Reading a large file, or sorting and filtering in the background.
    func updateBusyState() {
        let stack = busyLabel.superview
        updateSheetTabs()
        if let loading = workbook?.loadingSheet {
            let percent = loading.fraction.formatted(.percent.precision(.fractionLength(0)))
            busyLabel.stringValue = L("Reading “%@”: %@", loading.name, percent)
            busyCancel.isHidden = false
        } else if let fraction = document.indexingFraction {
            busyLabel.stringValue = L("Reading the file: %@", fraction.formatted(.percent.precision(.fractionLength(0))))
            busyCancel.isHidden = true
        } else if let status = document.editStatus {
            let fraction = document.editFraction ?? 0
            busyLabel.stringValue = status + " " + fraction.formatted(.percent.precision(.fractionLength(0)))
            busyCancel.isHidden = false
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

    /// Stops the change, or else the sorting and filtering, running in the background.
    @objc func cancelBackgroundWork(_ sender: Any?) {
        if let workbook, workbook.loadingSheet != nil {
            workbook.cancelSheetLoading()
        } else if document.editStatus != nil {
            document.cancelEdit()
        } else {
            document.cancelViewUpdate()
        }
    }

    // Undo and redo wait for a change running in the background: the core takes no other
    // changes until it ends. A field editor handles its own undo before these.

    @objc func undo(_ sender: Any?) {
        document.undoManager?.undo()
    }

    @objc func redo(_ sender: Any?) {
        document.undoManager?.redo()
    }

    func validateUndo(_ menuItem: NSMenuItem) -> Bool {
        guard let undoManager = document.undoManager else { return false }
        if menuItem.action == #selector(undo(_:)) {
            menuItem.title = undoManager.undoMenuItemTitle
            return !document.isBusy && undoManager.canUndo && core.canUndo()
        }
        menuItem.title = undoManager.redoMenuItemTitle
        return !document.isBusy && undoManager.canRedo
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

    /// Commands that change the file, which wait until a large file is read whole and until a
    /// change running in the background ends.
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

    /// Commands that insert, delete or move columns, which a workbook does not take yet.
    static let columnActions: Set<Selector> = [
        #selector(insertColumnLeft(_:)), #selector(insertColumnRight(_:)), #selector(deleteColumn(_:)),
        #selector(moveColumnLeft(_:)), #selector(moveColumnRight(_:)), #selector(splitColumn(_:)),
        #selector(mergeColumns(_:)),
    ]

    /// Commands that insert, delete or move rows inside a sheet, which a sheet whose rows other
    /// parts of the workbook refer to does not take. Rows can still be added at the end.
    static let rowActions: Set<Selector> = [
        #selector(deleteRows(_:)), #selector(moveRowsUp(_:)), #selector(moveRowsDown(_:)),
        #selector(applySortToFile(_:)), #selector(removeDuplicateRows(_:)),
    ]

    /// Commands that change the file, which a document open for viewing only does not take.
    /// Sorting, filtering and the header setting only change what is shown.
    static let editingActions: Set<Selector> = changingActions.subtracting([
        #selector(sortAscending(_:)), #selector(sortDescending(_:)), #selector(clearSort(_:)),
        #selector(toggleFilterBar(_:)), #selector(clearFilter(_:)), #selector(toggleHeaderRow(_:)),
    ]).union([
        #selector(toggleRecipeRecording(_:)), #selector(applySavedRecipe(_:)), #selector(showOpenOptions(_:)),
    ])
}
