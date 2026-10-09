//! Saving workbooks: only what changed is written anew.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use comma_core::{DisplayLocale, Document, Error, SheetStructure, Workbook};
use rust_xlsxwriter::{Format, Formula};

fn russian() -> DisplayLocale {
    DisplayLocale {
        decimal_separator: ',',
        group_separator: '\u{a0}',
        short_date: "dd.mm.yyyy".to_owned(),
        ..DisplayLocale::default()
    }
}

/// Parts of an archive by name, decompressed.
fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            (file.name().to_owned(), data)
        })
        .collect()
}

fn text(bytes: &[u8], part: &str) -> String {
    String::from_utf8(parts(bytes)[part].clone()).unwrap()
}

fn rows(document: &Document) -> Vec<Vec<String>> {
    (0..document.row_count()).map(|row| document.row(row).unwrap()).collect()
}

/// A workbook without formulas, whose rows can move.
fn plain_book() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let money = Format::new().set_num_format("#,##0.00\\ \"₽\"");
    let plain = workbook.add_worksheet();
    plain.set_name("Plain").unwrap();
    plain.write_string(0, 0, "Name").unwrap();
    plain.write_string(0, 1, "Amount").unwrap();
    for (row, (name, amount)) in [("Ann", 30.0), ("Bob", 10.0), ("Cid", 20.0)].iter().enumerate() {
        plain.write_string(row as u32 + 1, 0, *name).unwrap();
        plain.write_number_with_format(row as u32 + 1, 1, *amount, &money).unwrap();
    }
    workbook.save_to_buffer().unwrap()
}

/// A sheet with styles and a formula, and a plain sheet with styles only.
fn book() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let money = Format::new().set_num_format("#,##0.00\\ \"₽\"");
    let date = Format::new().set_num_format("dd.mm.yyyy");
    let bold = Format::new().set_bold();
    let sales = workbook.add_worksheet();
    sales.set_name("Sales").unwrap();
    sales.write_string_with_format(0, 0, "Item", &bold).unwrap();
    sales.write_string_with_format(0, 1, "Price", &bold).unwrap();
    sales.write_string_with_format(0, 2, "Date", &bold).unwrap();
    sales.write_string_with_format(0, 3, "Total", &bold).unwrap();
    for (row, (item, price)) in [("Tea", 250.5), ("Coffee", 890.0), ("Sugar", 75.25)].iter().enumerate() {
        let row = row as u32 + 1;
        sales.write_string(row, 0, *item).unwrap();
        sales.write_number_with_format(row, 1, *price, &money).unwrap();
        sales.write_number_with_format(row, 2, 45658.0 + f64::from(row), &date).unwrap();
    }
    sales.write_formula_with_format(1, 3, Formula::new("=B2*2").set_result("501"), &money).unwrap();
    let plain = workbook.add_worksheet();
    plain.set_name("Plain").unwrap();
    plain.write_string(0, 0, "Name").unwrap();
    plain.write_string(0, 1, "Amount").unwrap();
    for (row, (name, amount)) in [("Ann", 30.0), ("Bob", 10.0), ("Cid", 20.0)].iter().enumerate() {
        plain.write_string(row as u32 + 1, 0, *name).unwrap();
        plain.write_number_with_format(row as u32 + 1, 1, *amount, &money).unwrap();
    }
    workbook.save_to_buffer().unwrap()
}

#[test]
fn saving_without_changes_keeps_every_part() {
    let original = book();
    let workbook = Workbook::from_bytes(original.clone()).unwrap();
    let sales = workbook.read_sheet(0, &russian()).unwrap().document;
    let plain = workbook.read_sheet(1, &russian()).unwrap().document;
    let saved = workbook.to_bytes(&[(0, &sales), (1, &plain)]).unwrap();
    assert_eq!(parts(&saved), parts(&original));
}

#[test]
fn an_edited_cell_changes_only_its_row() {
    let original = book();
    let workbook = Workbook::from_bytes(original.clone()).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    // Typed into a money cell, as it shows; then a plain number into a date cell.
    sales.set_cell(2, 1, "1\u{a0}000,00 ₽").unwrap();
    sales.set_cell(2, 2, "2025-02-01").unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();

    let before = parts(&original);
    let after = parts(&saved);
    let changed: Vec<_> = after.keys().filter(|name| before.get(*name) != after.get(*name)).cloned().collect();
    // rust_xlsxwriter already asks Excel to compute on load, so workbook.xml stays too.
    assert_eq!(changed, ["xl/worksheets/sheet1.xml"]);
    let sheet = text(&saved, "xl/worksheets/sheet1.xml");
    let old_sheet = text(&original, "xl/worksheets/sheet1.xml");
    // Every row but row 3 is the same text as before.
    for row in ["<row r=\"1\"", "<row r=\"2\"", "<row r=\"4\""] {
        let piece = |xml: &str| {
            let start = xml.find(row).unwrap();
            xml[start..start + xml[start..].find("</row>").unwrap()].to_owned()
        };
        assert_eq!(piece(&sheet), piece(&old_sheet), "{row}");
    }
    assert!(text(&saved, "xl/workbook.xml").contains("fullCalcOnLoad=\"1\""));

    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &russian()).unwrap().document;
    assert_eq!(reopened.row(2).unwrap()[..3], ["Coffee", "1\u{a0}000,00 ₽", "01.02.2025"]);
    assert_eq!(reopened.formula(1, 3).as_deref(), Some("B2*2"));
}

#[test]
fn values_keep_their_types_and_text_round_trips() {
    let workbook = Workbook::from_bytes(book()).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    let tricky = " a < b & c _x0041_\r\nline ";
    sales.set_cell(1, 0, tricky).unwrap();
    sales.set_cell(3, 0, "007").unwrap();
    sales.set_cell(3, 1, "").unwrap();
    sales.set_cell(1, 3, "999").unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let sheet = text(&saved, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<c r="A4" t="inlineStr"><is><t>007</t></is></c>"#), "{sheet}");
    // A cleared cell keeps its style.
    assert!(sheet.contains(r#"<c r="B4" s="#), "{sheet}");
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &russian()).unwrap().document;
    assert_eq!(reopened.cell(1, 0).unwrap(), tricky);
    assert_eq!(reopened.cell(3, 0).unwrap(), "007");
    assert_eq!(reopened.cell(3, 1).unwrap(), "");
    // The formula was typed over: a plain value in the money format now.
    assert_eq!(reopened.formula(1, 3), None);
    assert_eq!(reopened.cell(1, 3).unwrap(), "999,00 ₽");
}

#[test]
fn rows_of_a_plain_sheet_move_with_their_styles() {
    let workbook = Workbook::from_bytes(plain_book()).unwrap();
    let mut plain = workbook.read_sheet(0, &russian()).unwrap().document;
    assert!(!plain.sheet_rows_fixed());
    // The first row taken as column names stays on top when the rows are sorted.
    plain.set_has_header(true);
    plain.set_view(comma_core::ViewSpec { sort: vec![comma_core::SortKey { column: 1, ascending: true }], filters: vec![] }).unwrap();
    assert!(plain.apply_sort().unwrap());
    plain.delete_rows(&[2]).unwrap();
    plain.insert_rows(1, 1).unwrap();
    plain.set_cell(1, 0, "New").unwrap();
    plain.set_cell(1, 1, "5").unwrap();
    let expected = rows(&plain);
    let saved = workbook.to_bytes(&[(0, &plain)]).unwrap();
    let sheet = text(&saved, "xl/worksheets/sheet1.xml");
    let reopened = Workbook::from_bytes(saved.clone()).unwrap().read_sheet(0, &russian()).unwrap().document;
    assert_eq!(rows(&reopened), expected);
    assert_eq!(expected[1..], [vec!["New", "5"], vec!["Bob", "10,00 ₽"], vec!["Ann", "30,00 ₽"]]);
    // Ann's amount moved from row 2 to row 4 and kept its style.
    assert!(sheet.contains(r#"<c r="B4" s="1"><v>30</v></c>"#), "{sheet}");
}

#[test]
fn rows_others_refer_to_stay_in_place() {
    let workbook = Workbook::from_bytes(book()).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    assert!(sales.sheet_rows_fixed());
    assert!(matches!(sales.insert_rows(1, 1), Err(Error::SheetStructure(SheetStructure::RowsFixed))));
    assert!(matches!(sales.delete_rows(&[1]), Err(Error::SheetStructure(SheetStructure::RowsFixed))));
    assert!(matches!(sales.insert_column(1), Err(Error::SheetStructure(SheetStructure::ColumnsFixed))));
    // The sheet without formulas is fixed too: the formula on the other sheet may refer to it.
    assert!(workbook.read_sheet(1, &russian()).unwrap().document.sheet_rows_fixed());
    // Rows can still be added at the end.
    let end = sales.row_count();
    sales.insert_rows(end, 1).unwrap();
    sales.set_cell(end, 0, "Salt").unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &russian()).unwrap().document;
    assert_eq!(reopened.cell(end, 0).unwrap(), "Salt");
}

#[test]
fn undoing_every_change_saves_the_file_as_it_was() {
    let original = book();
    let workbook = Workbook::from_bytes(original.clone()).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    sales.set_cell(1, 0, "Changed").unwrap();
    assert_ne!(parts(&workbook.to_bytes(&[(0, &sales)]).unwrap()), parts(&original));
    sales.undo();
    assert_eq!(parts(&workbook.to_bytes(&[(0, &sales)]).unwrap()), parts(&original));
}

#[test]
fn a_formula_typed_over_takes_the_calculation_chain_away() {
    // rust_xlsxwriter writes no calculation chain; add one as Excel would.
    let mut archive = zip::ZipArchive::new(Cursor::new(book())).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().to_owned();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();
        if name == "[Content_Types].xml" {
            data = data.replace("</Types>", r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#);
        }
        if name == "xl/_rels/workbook.xml.rels" {
            data = data.replace("</Relationships>", r#"<Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#);
        }
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    writer.start_file("xl/calcChain.xml", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(br#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="D2" i="1"/></calcChain>"#).unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    let workbook = Workbook::from_bytes(bytes).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    // A value change keeps the chain.
    sales.set_cell(1, 1, "300").unwrap();
    assert!(parts(&workbook.to_bytes(&[(0, &sales)]).unwrap()).contains_key("xl/calcChain.xml"));
    sales.set_cell(1, 3, "1").unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let after = parts(&saved);
    assert!(!after.contains_key("xl/calcChain.xml"));
    assert!(!text(&saved, "[Content_Types].xml").contains("calcChain"));
    assert!(!text(&saved, "xl/_rels/workbook.xml.rels").contains("calcChain"));
}
