//! Inserting and deleting rows and columns of a sheet of a workbook, with every reference to
//! the cells that move moved too: in formulas of all sheets here, and, when saving, in merged
//! cells, conditional formats, validations, links, filters and defined names.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Workbook;
use super::refs::{Remap, remap_formula};
use crate::document::{Document, Field, FormulaField, NO_ORIGIN, Record};
use crate::error::{Error, Result, SheetStructure};
use crate::history::Splice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Columns,
}

impl Workbook {
    /// Inserts `count` empty rows or columns before `at` in sheet `index` (a document among
    /// `sheets`, which must hold every sheet with formulas), moving references to the cells after
    /// them. Each document changed takes one undoable step; returns the sheets changed.
    pub fn insert(&self, sheets: &mut [(usize, &mut Document)], index: usize, axis: Axis, at: usize, count: usize) -> Result<Vec<usize>> {
        self.restructure(sheets, index, axis, at, count, true, None)
    }

    /// Splits the values of `column` of sheet `index` by `separator` (a regular expression
    /// when `regex`) into new columns right after it, as `Document::split_column` does, with
    /// references to the columns after it moved in every sheet; one undoable step. Returns
    /// the sheets changed and the number of columns added.
    pub fn split_column(
        &self,
        sheets: &mut [(usize, &mut Document)],
        index: usize,
        column: usize,
        separator: &str,
        regex: bool,
    ) -> Result<(Vec<usize>, usize)> {
        let Some(position) = sheets.iter().position(|(sheet, _)| *sheet == index) else { return Ok((Vec::new(), 0)) };
        let Some((count, values)) = sheets[position].1.split_values(column, separator, regex)? else { return Ok((Vec::new(), 0)) };
        let changed = self.restructure(sheets, index, Axis::Columns, column + 1, count, true, Some(&values))?;
        Ok((changed, count))
    }

    /// Joins the non-empty values of `columns` of sheet `index` with `separator` into a new
    /// column after the last of them, as `Document::merge_columns` does, with references to the
    /// columns after it moved in every sheet; one undoable step. Returns the sheets changed.
    pub fn merge_columns(&self, sheets: &mut [(usize, &mut Document)], index: usize, columns: &[usize], separator: &str) -> Result<Vec<usize>> {
        let Some(position) = sheets.iter().position(|(sheet, _)| *sheet == index) else { return Ok(Vec::new()) };
        let Some((at, values)) = sheets[position].1.merge_values(columns, separator)? else { return Ok(Vec::new()) };
        self.restructure(sheets, index, Axis::Columns, at, 1, true, Some(&values))
    }

    /// Deletes `count` rows or columns from `at` in sheet `index`; see `insert`. References to
    /// cells deleted become `#REF!`.
    pub fn delete(&self, sheets: &mut [(usize, &mut Document)], index: usize, axis: Axis, at: usize, count: usize) -> Result<Vec<usize>> {
        self.restructure(sheets, index, axis, at, count, false, None)
    }

    /// Inserts or deletes rows or columns; columns inserted take `fill`, by row, if given.
    #[allow(clippy::too_many_arguments)]
    fn restructure(
        &self,
        sheets: &mut [(usize, &mut Document)],
        index: usize,
        axis: Axis,
        at: usize,
        count: usize,
        insert: bool,
        fill: Option<&[Vec<Field>]>,
    ) -> Result<Vec<usize>> {
        let name = self.sheets.get(index).map(|sheet| sheet.name.to_lowercase()).ok_or(Error::RowOutOfRange { row: index, rows: self.sheets.len() })?;
        let Some(position) = sheets.iter().position(|(sheet, _)| *sheet == index) else {
            return Ok(Vec::new());
        };
        if sheets[position].1.sheet_structure_locked() {
            return Err(Error::SheetStructure(sheets[position].1.locked_structure(axis == Axis::Rows)));
        }
        let (length, limit) = match axis {
            Axis::Rows => (sheets[position].1.row_count(), super::refs::MAX_ROWS),
            Axis::Columns => (sheets[position].1.column_count(), super::refs::MAX_COLUMNS),
        };
        if count == 0 || at > length || (!insert && at + count > length) || (insert && length + count > limit as usize) {
            return Ok(Vec::new());
        }
        // Tables of the sheet: rows inserted among their data rows fill their calculated
        // columns, columns inserted into them get names; what would break a table is refused.
        let change = self.table_change(sheets, position, index, axis, at, count, insert)?;
        let (at32, count32) = (at as u32, count as u32);
        let shift = move |first: u32, last: u32| Remap::shift(first, last, at32, count32, insert);
        let same = |first: u32, last: u32| Some((first, last));
        let remap = match axis {
            Axis::Rows => Remap { sheet: &name, rows: &shift, columns: &same },
            Axis::Columns => Remap { sheet: &name, rows: &same, columns: &shift },
        };

        let mut changed = Vec::new();
        for (sheet, document) in sheets.iter_mut() {
            let own = self.sheets.get(*sheet).map(|entry| entry.name.to_lowercase()).unwrap_or_default();
            if *sheet == index {
                restructure_sheet(document, &own, &remap, axis, at, count, insert, &change, fill);
                changed.push(*sheet);
            } else if remap_formulas(document, &own, &remap) {
                changed.push(*sheet);
            }
        }

        // Rows edited before all count as changed, wherever they went, so that what reads
        // them is computed again.
        if axis == Axis::Rows {
            self.calc.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).entered.remove(&index);
        }
        Ok(changed)
    }
}

impl Workbook {
    /// Moves rows of sheet `index` to stand before row `to`, as cutting and inserting them
    /// does in Excel: references to them follow, in formulas of every sheet here and, when
    /// saving, in merged cells, formats, validations, links and names. One undoable step per
    /// document changed; returns the sheets changed.
    pub fn move_rows(&self, sheets: &mut [(usize, &mut Document)], index: usize, rows: &[usize], to: usize) -> Result<Vec<usize>> {
        let name = self.sheets.get(index).map(|sheet| sheet.name.to_lowercase()).ok_or(Error::RowOutOfRange { row: index, rows: self.sheets.len() })?;
        let Some(position) = sheets.iter().position(|(sheet, _)| *sheet == index) else {
            return Ok(Vec::new());
        };
        let document = &sheets[position].1;
        if document.sheet_structure_locked() {
            return Err(Error::SheetStructure(document.locked_structure(true)));
        }
        let mut rows = rows.to_vec();
        rows.sort_unstable();
        rows.dedup();
        let length = document.row_count();
        let Some(&last) = rows.last() else { return Ok(Vec::new()) };
        if last >= length || to > length {
            return Err(Error::RowOutOfRange { row: last.max(to), rows: length });
        }
        // The new order of the rows, and where each old row goes.
        let target = to - rows.iter().filter(|&&row| row < to).count();
        let moving: std::collections::HashSet<usize> = rows.iter().copied().collect();
        let mut order: Vec<usize> = (0..length).filter(|row| !moving.contains(row)).collect();
        order.splice(target..target, rows.iter().copied());
        if order.iter().enumerate().all(|(new, &old)| new == old) {
            return Ok(Vec::new());
        }
        self.check_table_rows(document, index, &order)?;
        // A merged area over several rows moves whole or not at all, as Excel has it, and no
        // row comes in among its rows.
        if let Some(sheet) = &document.sheet {
            let spans: Vec<(usize, usize)> = sheet.merged.iter().map(|area| (area.top, area.bottom)).collect();
            if !keeps_together(&spans, &order, |row| document.row_origin(row)) {
                return Err(Error::SheetStructure(SheetStructure::MergedArea));
            }
        }
        let mut new_of_old = vec![0u32; length];
        for (new, &old) in order.iter().enumerate() {
            new_of_old[old] = new as u32;
        }
        let place = |row: u32| new_of_old.get(row as usize).copied().unwrap_or(row);
        // A reference follows its rows; a range keeps its ends, or, when rows moved within it
        // turned them around, covers where its rows went.
        let moved = |first: u32, last: u32| {
            let (top, bottom) = (place(first), place(last));
            if top <= bottom {
                return Some((top, bottom));
            }
            let end = last.min(length as u32 - 1);
            let (low, high) = (first..=end).map(place).fold((u32::MAX, 0), |(low, high), row| (low.min(row), high.max(row)));
            Some((low, if last > end { last } else { high }))
        };
        let same = |first: u32, last: u32| Some((first, last));
        let remap = Remap { sheet: &name, rows: &moved, columns: &same };

        let mut changed = Vec::new();
        for (sheet, document) in sheets.iter_mut() {
            let own = self.sheets.get(*sheet).map(|entry| entry.name.to_lowercase()).unwrap_or_default();
            if *sheet != index {
                if remap_formulas(document, &own, &remap) {
                    changed.push(*sheet);
                }
                continue;
            }
            let mut formulas_moved = false;
            let records: Vec<Record> = order
                .iter()
                .map(|&old| {
                    let record = &document.records[old];
                    match remapped(&document.fields(record), &own, &remap) {
                        Some(fields) => {
                            formulas_moved = true;
                            record.with_fields(fields)
                        }
                        None => record.clone(),
                    }
                })
                .collect();
            let columns = document.column_count();
            document.replace_records(records, columns, true);
            document.refresh_view();
            if formulas_moved {
                mark_formulas_moved(document);
            }
            changed.push(*sheet);
        }
        self.calc.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).entered.remove(&index);
        Ok(changed)
    }
}

impl Workbook {
    /// Moves column `from` of sheet `index` to stand at `to` among the columns after it, as
    /// cutting and inserting a column does in Excel: references to it and to the columns it
    /// passes follow, in formulas of every sheet here and, when saving, in widths, merged
    /// cells, formats and names. Refused when it would split a merged area or move columns of
    /// an Excel table. One undoable step per document changed; returns the sheets changed.
    pub fn move_column(&self, sheets: &mut [(usize, &mut Document)], index: usize, from: usize, to: usize) -> Result<Vec<usize>> {
        let name = self.sheets.get(index).map(|sheet| sheet.name.to_lowercase()).ok_or(Error::RowOutOfRange { row: index, rows: self.sheets.len() })?;
        let Some(position) = sheets.iter().position(|(sheet, _)| *sheet == index) else {
            return Ok(Vec::new());
        };
        let document = &*sheets[position].1;
        if document.sheet_structure_locked() {
            return Err(Error::SheetStructure(document.locked_structure(false)));
        }
        let length = document.column_count();
        if from >= length || to >= length {
            return Err(Error::ColumnOutOfRange { column: from.max(to), columns: length });
        }
        if from == to {
            return Ok(Vec::new());
        }
        // The columns that change places: the one moved and those it passes.
        let (low, high) = (from.min(to), from.max(to));
        for table in self.tables().iter().filter(|table| table.sheet == index).filter_map(|table| table.current(document)) {
            if (table.left as usize) <= high && (table.right as usize) >= low {
                return Err(Error::SheetStructure(SheetStructure::TableColumnsMoved));
            }
        }
        let mut order: Vec<usize> = (0..length).collect();
        let column = order.remove(from);
        order.insert(to, column);
        // A merged area stays together, its columns side by side in their order.
        if let Some(sheet) = &document.sheet {
            let spans: Vec<(usize, usize)> = sheet.merged.iter().map(|area| (area.left, area.right)).collect();
            if !keeps_together(&spans, &order, |column| document.column_origin(column)) {
                return Err(Error::SheetStructure(SheetStructure::MergedArea));
            }
        }
        let mut new_of_old = vec![0u32; length];
        for (new, &old) in order.iter().enumerate() {
            new_of_old[old] = new as u32;
        }
        let place = |column: u32| new_of_old.get(column as usize).copied().unwrap_or(column);
        // A reference follows its columns; a range keeps its ends, or, when columns moved
        // within it turned them around, covers where its columns went.
        let moved = |first: u32, last: u32| {
            let (left, right) = (place(first), place(last));
            if left <= right {
                return Some((left, right));
            }
            let end = last.min(length as u32 - 1);
            let (low, high) = (first..=end).map(place).fold((u32::MAX, 0), |(low, high), column| (low.min(column), high.max(column)));
            Some((low, if last > end { last } else { high }))
        };
        let same = |first: u32, last: u32| Some((first, last));
        let remap = Remap { sheet: &name, rows: &same, columns: &moved };

        let mut changed = Vec::new();
        for (sheet, document) in sheets.iter_mut() {
            let own = self.sheets.get(*sheet).map(|entry| entry.name.to_lowercase()).unwrap_or_default();
            if *sheet != index {
                if remap_formulas(document, &own, &remap) {
                    changed.push(*sheet);
                }
                continue;
            }
            let records: Vec<Record> = document
                .records
                .iter()
                .map(|record| {
                    let mut fields = document.fields(record);
                    if let Some(moved) = remapped(&fields, &own, &remap) {
                        fields = moved;
                    }
                    if !record.is_blank() && fields.len() > low {
                        if fields.len() <= high {
                            fields.resize(high + 1, Field::Text(String::new()));
                        }
                        let field = fields.remove(from);
                        fields.insert(to, field);
                    }
                    record.with_fields(fields)
                })
                .collect();
            let mut origins = (*document.column_origins).clone();
            if origins.len() <= high {
                origins.resize(high + 1, NO_ORIGIN);
            }
            let origin = origins.remove(from);
            origins.insert(to, origin);
            let rows = document.records.len();
            let splice = Splice::apply(Arc::make_mut(&mut document.records), 0, rows, records);
            document.commit_moving_columns(vec![splice], length, origins);
            document.reset_view();
            mark_formulas_moved(document);
            changed.push(*sheet);
        }
        Ok(changed)
    }
}

impl Workbook {
    /// Puts the rows of sheet `index` in the order its view sorts them, as Excel sorts: rows
    /// take their values and formulas along, a formula's relative references moving with it as
    /// when copied, while formats and every reference from elsewhere stay where they are.
    /// Refused when a merged area lies among the rows that move, as Excel refuses it. One
    /// undoable step; `false` when nothing moved.
    pub fn sort_rows(&self, document: &mut Document, index: usize) -> Result<bool> {
        if document.sheet_structure_locked() {
            return Err(Error::SheetStructure(document.locked_structure(true)));
        }
        let Some(order) = document.sorted_order() else { return Ok(false) };
        if order.iter().enumerate().all(|(new, &old)| new == old) {
            document.clear_sort();
            return Ok(false);
        }
        self.check_table_rows(document, index, &order)?;
        let moving: Vec<usize> = order.iter().enumerate().filter(|(new, old)| new != *old).map(|(new, _)| new).collect();
        if let Some(sheet) = &document.sheet {
            let origins: std::collections::HashSet<usize> = moving.iter().filter_map(|&row| document.row_origin(row)).collect();
            if sheet.merged.iter().any(|area| (area.top..=area.bottom).any(|row| origins.contains(&row))) {
                return Err(Error::SheetStructure(SheetStructure::MergedArea));
            }
        }
        let locale = document.sheet.as_ref().map(|sheet| sheet.locale.clone()).unwrap_or_default();
        let values = self.file_values(index, &locale);
        let mut records = Vec::with_capacity(order.len());
        for (new, &old) in order.iter().enumerate() {
            let record = &document.records[old];
            if new == old {
                records.push(record.clone());
                continue;
            }
            let delta = new as i64 - old as i64;
            let origin = document.row_origin(old);
            let fields = document
                .fields(record)
                .into_iter()
                .enumerate()
                .map(|(column, field)| match field {
                    Field::Formula(cell) => Field::Formula(Box::new(FormulaField {
                        formula: super::shift_formula(&cell.formula, delta, 0),
                        array: cell.array.as_deref().map(|range| super::shift_formula(range, delta, 0)),
                        from_file: false,
                        ..*cell
                    })),
                    // A value of the file keeps its exact value, not the one shown rounded.
                    Field::Text(text) => {
                        let exact = origin
                            .zip(document.column_origin(column))
                            .and_then(|(row, column)| values.as_ref()?.exact(row, column, &text));
                        match exact {
                            Some(input) if input != text => {
                                Field::Entered(Box::new(crate::document::EnteredField { input, shown: text, exact: true }))
                            }
                            _ => Field::Text(text),
                        }
                    }
                    field => field,
                })
                .collect();
            // The row stands at the place of the file's row there, with its formats.
            records.push(record.at_origin(fields, document.row_origin(new)));
        }
        let columns = document.column_count();
        document.replace_records(records, columns, true);
        document.clear_sort();
        mark_formulas_moved(document);
        Ok(true)
    }
}

/// The formulas of a record with references moved; `None` if none changed.
fn remapped(fields: &[Field], own: &str, remap: &Remap) -> Option<Vec<Field>> {
    let mut changed = false;
    let fields: Vec<Field> = fields
        .iter()
        .map(|field| match field {
            Field::Formula(cell) => {
                let formula = remap_formula(&cell.formula, own, remap);
                let array = cell.array.as_ref().map(|range| remap_formula(range, own, remap));
                if formula == cell.formula && array == cell.array {
                    return field.clone();
                }
                changed = true;
                Field::Formula(Box::new(FormulaField { formula, array, from_file: false, ..(**cell).clone() }))
            }
            field => field.clone(),
        })
        .collect();
    changed.then_some(fields)
}

/// Moves references in the formulas of a sheet other than the one that changed. Returns
/// whether any moved.
fn remap_formulas(document: &mut Document, own: &str, remap: &Remap) -> bool {
    let mut splices = Vec::new();
    let rows: Vec<usize> = document.formula_cells().map(|(row, _, _)| row).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    for row in rows {
        let record = &document.records[row];
        let Some(fields) = remapped(&document.fields(record), own, remap) else { continue };
        let updated = record.with_fields(fields);
        splices.push(Splice::apply(Arc::make_mut(&mut document.records), row, 1, vec![updated]));
    }
    if splices.is_empty() {
        return false;
    }
    let columns = document.column_count();
    document.commit(splices, columns, false);
    mark_formulas_moved(document);
    true
}

impl Workbook {
    /// Checks a change of rows or columns against the Excel tables of sheet `index`, where
    /// they are now: a column inserted or deleted inside a table, or rows deleted from its
    /// headers or totals or all its data, are refused. Rows inserted among a table's data rows
    /// get its calculated columns: the fields they start with.
    /// Fails if rows going to new places, `order` giving the old row of each place, would
    /// break an Excel table of the sheet: a row that moves must stay among the data rows of
    /// its table, or outside every table, and a row moving among a table's data must have no
    /// cells beside the table, which Excel would leave where they are.
    fn check_table_rows(&self, document: &Document, index: usize, order: &[usize]) -> Result<()> {
        // Each table's rows, data rows and columns where they are now.
        let tables: Vec<(usize, usize, usize, usize, usize, usize)> = self
            .tables()
            .iter()
            .filter(|table| table.sheet == index)
            .filter_map(|table| table.current(document))
            .map(|table| {
                let (top, bottom) = (table.top as usize, table.bottom as usize);
                (top, bottom, top + table.headers as usize, bottom.saturating_sub(table.totals as usize), table.left as usize, table.right as usize)
            })
            .collect();
        if tables.is_empty() {
            return Ok(());
        }
        // Where a row stands: among the data of a table, on its headers or totals, or outside.
        let place = |row: usize| -> Option<(usize, bool)> {
            tables.iter().position(|&(top, bottom, ..)| (top..=bottom).contains(&row)).map(|table| {
                let (_, _, first, last, ..) = tables[table];
                (table, (first..=last).contains(&row))
            })
        };
        for (new, &old) in order.iter().enumerate().filter(|(new, old)| *new != **old) {
            let (from, to) = (place(old), place(new));
            let stays = match (from, to) {
                (None, None) => true,
                (Some((table, true)), Some((other, true))) if table == other => {
                    let (.., left, right) = tables[table];
                    document.row_fields(old).iter().enumerate().all(|(column, field)| (left..=right).contains(&column) || super::save::value(field).is_empty())
                }
                _ => false,
            };
            if !stays {
                return Err(Error::SheetStructure(SheetStructure::TableRowsMoved));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn table_change(
        &self,
        sheets: &[(usize, &mut Document)],
        position: usize,
        index: usize,
        axis: Axis,
        at: usize,
        count: usize,
        insert: bool,
    ) -> Result<TableChange> {
        let document = &*sheets[position].1;
        let mut change = TableChange::default();
        let last = at + count - 1;
        for table in self.tables().iter().filter(|table| table.sheet == index).filter_map(|table| table.current(document)) {
            let (top, bottom, left, right) = (table.top as usize, table.bottom as usize, table.left as usize, table.right as usize);
            let (first_data, last_data) = (top + table.headers as usize, bottom.saturating_sub(table.totals as usize));
            match axis {
                // A column inserted into a table is a column of it, named as Excel names one.
                Axis::Columns if insert && at > left && at <= right => {
                    let word = column_word(document);
                    let mut names: Vec<String> = table.columns.clone();
                    for offset in 0..count {
                        let name = (1..).map(|number| format!("{word}{number}")).find(|name| !names.contains(&name.to_lowercase())).unwrap_or_default();
                        names.push(name.to_lowercase());
                        if table.headers > 0 {
                            change.headers.push((top, at + offset, name));
                        }
                    }
                }
                Axis::Columns if !insert && at <= right && last >= left => {
                    if at <= left && last >= right {
                        return Err(Error::SheetStructure(SheetStructure::TableColumns));
                    }
                    // Formulas naming a column deleted would read nothing Excel could tell.
                    let deleted: Vec<&String> = (at.max(left)..=last.min(right)).filter_map(|column| table.columns.get(column - left)).collect();
                    let names_one = |formula: &str| {
                        let formula = formula.to_lowercase();
                        deleted.iter().any(|name| formula.contains(&format!("[{name}]")) || formula.contains(&format!("@{name}")) || formula.contains(&format!(",{name}]")))
                    };
                    let kept_formulas = table.formulas.iter().enumerate().filter(|(offset, _)| !(at..=last).contains(&(left + offset))).filter_map(|(_, formula)| formula.as_deref());
                    // Formulas in the columns deleted go with them.
                    let in_cells = sheets.iter().any(|(sheet, document)| {
                        document.formula_cells().any(|(_, column, cell)| !(*sheet == index && (at..=last).contains(&column)) && names_one(&cell.formula))
                    });
                    if in_cells || kept_formulas.into_iter().any(names_one) {
                        return Err(Error::SheetStructure(SheetStructure::TableColumnInUse));
                    }
                }
                Axis::Rows if !insert => {
                    let headers = table.headers > 0 && at < first_data && last >= top;
                    let totals = table.totals > 0 && at <= bottom && last > last_data;
                    let all_data = at <= first_data && last >= last_data;
                    if (headers || totals || all_data) && at <= bottom && last >= top {
                        return Err(Error::SheetStructure(SheetStructure::TableEdges));
                    }
                }
                Axis::Rows if insert && at >= first_data && at <= last_data + 1 && (table.totals > 0 || at <= last_data) => {
                    let fields = change.template.get_or_insert_with(Vec::new);
                    for (offset, formula) in table.formulas.iter().enumerate() {
                        let Some(formula) = formula else { continue };
                        let column = left + offset;
                        if fields.len() <= column {
                            fields.resize(column + 1, Field::Text(String::new()));
                        }
                        fields[column] = Field::Formula(Box::new(FormulaField {
                            formula: formula.clone(),
                            value: String::new(),
                            array: None,
                            computed: crate::document::Computed::Unknown,
                            from_file: false,
                            date_format: None,
                        }));
                    }
                }
                _ => {}
            }
        }
        Ok(change)
    }
}

/// Whether spans of rows or columns of the file, merged areas, stay side by side in their
/// order once the document's rows or columns stand in `order` (the old place of each new one);
/// `origin` gives the row or column of the file at an old place.
fn keeps_together(spans: &[(usize, usize)], order: &[usize], origin: impl Fn(usize) -> Option<usize>) -> bool {
    let wide: Vec<&(usize, usize)> = spans.iter().filter(|(first, last)| last > first).collect();
    if wide.is_empty() {
        return true;
    }
    // Where each place of the file stands now.
    let mut now = std::collections::HashMap::new();
    for (new, &old) in order.iter().enumerate() {
        if let Some(origin) = origin(old) {
            now.insert(origin, new);
        }
    }
    wide.iter().all(|&&(first, last)| {
        let places: Vec<Option<&usize>> = (first..=last).map(|place| now.get(&place)).collect();
        places.iter().all(Option::is_some) && places.windows(2).all(|pair| *pair[1].unwrap() == *pair[0].unwrap() + 1)
    })
}

/// What tables of a sheet add to rows or columns inserted: the cells of a row inserted among
/// their data, and the names of columns inserted into them, by row and column.
#[derive(Default)]
struct TableChange {
    template: Option<Vec<Field>>,
    headers: Vec<(usize, usize, String)>,
}

/// The word Excel names a new column of a table with, in the language the sheet shows.
fn column_word(document: &Document) -> &'static str {
    match document.sheet.as_ref() {
        Some(sheet) if sheet.locale.true_text == "ИСТИНА" => "Столбец",
        _ => "Column",
    }
}

/// Inserts or deletes rows or columns of the sheet that changed, with references in its own
/// formulas moved, as one undoable step. Rows inserted start with `template`, if given.
#[allow(clippy::too_many_arguments)]
fn restructure_sheet(
    document: &mut Document,
    own: &str,
    remap: &Remap,
    axis: Axis,
    at: usize,
    count: usize,
    insert: bool,
    change: &TableChange,
    fill: Option<&[Vec<Field>]>,
) {
    let template = change.template.as_deref();
    let mut formulas_moved = false;
    let mut records: Vec<Record> = document
        .records
        .iter()
        .enumerate()
        .map(|(row, record)| {
            let mut fields = document.fields(record);
            if let Some(moved) = remapped(&fields, own, remap) {
                fields = moved;
                formulas_moved = true;
            }
            if axis == Axis::Columns {
                let filled = fill.and_then(|fill| fill.get(row)).filter(|fields| fields.iter().any(|field| !super::save::value(field).is_empty()));
                if insert && let Some(filled) = filled {
                    if fields.len() < at {
                        fields.resize(at, Field::Text(String::new()));
                    }
                    fields.splice(at..at, (0..count).map(|offset| filled.get(offset).cloned().unwrap_or_else(|| Field::Text(String::new()))));
                } else if insert && fields.len() > at {
                    fields.splice(at..at, std::iter::repeat_n(Field::Text(String::new()), count));
                } else if !insert && fields.len() > at {
                    fields.drain(at..(at + count).min(fields.len()));
                }
            }
            record.with_fields(fields)
        })
        .collect();
    let columns = document.column_count();
    match axis {
        Axis::Rows => {
            if insert {
                let width = columns.max(1);
                let new = (0..count).map(|_| {
                    let mut fields = vec![Field::Text(String::new()); width];
                    for (column, field) in template.unwrap_or_default().iter().enumerate().take(width) {
                        fields[column] = field.clone();
                    }
                    Record::modified(fields, crate::parser::Terminator::Lf)
                });
                records.splice(at..at, new);
            } else {
                records.drain(at..at + count);
            }
            document.replace_records(records, columns, true);
            document.refresh_view();
        }
        Axis::Columns => {
            let mut origins = (*document.column_origins).clone();
            if insert {
                if at < origins.len() {
                    origins.splice(at..at, std::iter::repeat_n(NO_ORIGIN, count));
                }
            } else {
                let end = (at + count).min(origins.len());
                if at < end {
                    origins.drain(at..end);
                }
            }
            let after = if insert { columns + count } else { columns - count };
            // Columns inserted into a table get their names in its header row, unless what
            // fills them names them.
            for (row, column, name) in &change.headers {
                if let Some(record) = records.get_mut(*row) {
                    let mut fields = document.fields(record);
                    if fields.len() <= *column {
                        fields.resize(column + 1, Field::Text(String::new()));
                    }
                    if super::save::value(&fields[*column]).is_empty() {
                        fields[*column] = Field::Text(name.clone());
                        *record = record.with_fields(fields);
                    }
                }
            }
            let length = document.records.len();
            let splice = Splice::apply(Arc::make_mut(&mut document.records), 0, length, records);
            document.commit_moving_columns(vec![splice], after.max(1), origins);
            document.reset_view();
        }
    }
    if formulas_moved {
        mark_formulas_moved(document);
    }
}

fn mark_formulas_moved(document: &Document) {
    if let Some(sheet) = &document.sheet {
        sheet.formulas_moved.store(true, Ordering::Relaxed);
    }
}
