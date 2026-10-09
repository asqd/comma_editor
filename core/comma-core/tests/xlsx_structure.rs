//! Inserting and deleting rows and columns of workbooks, with references moved.

use std::io::{Cursor, Read, Write};

use comma_core::{Axis, DisplayLocale, Document, Error, SheetStructure, Workbook};
use rust_xlsxwriter::{
    ConditionalFormatFormula, DataValidation, DataValidationRule, Format, Formula, Table,
};

const NOW: f64 = 45_658.5;

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive.by_name(name).unwrap().read_to_string(&mut text).unwrap();
    text
}

/// Data: items and prices in rows 2–6, C = B * 2, a merged label and a total in row 8, a
/// conditional format and a validation on the prices, a wide column C, a defined name for
/// the prices. Report reads Data.
fn book() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let data = workbook.add_worksheet();
    data.set_name("Data").unwrap();
    data.write_string(0, 0, "Item").unwrap();
    data.write_string(0, 1, "Price").unwrap();
    data.write_string(0, 2, "Double").unwrap();
    for (index, price) in [10.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
        let row = index as u32 + 1;
        data.write_string(row, 0, format!("Item {row}")).unwrap();
        data.write_number(row, 1, *price).unwrap();
        data.write_formula(row, 2, Formula::new(format!("=B{}*2", row + 1)).set_result((price * 2.0).to_string())).unwrap();
    }
    data.merge_range(7, 0, 7, 1, "Total", &Format::new()).unwrap();
    data.write_formula(7, 2, Formula::new("=SUM(B2:B6)").set_result("150")).unwrap();
    data.add_conditional_format(1, 1, 5, 1, &ConditionalFormatFormula::new().set_rule("=$B2>25").set_format(Format::new().set_bold())).unwrap();
    data.add_data_validation(1, 1, 5, 1, &DataValidation::new().allow_whole_number(DataValidationRule::Between(0, 1000))).unwrap();
    data.set_column_width(2, 20).unwrap();
    let report = workbook.add_worksheet();
    report.set_name("Report").unwrap();
    report.write_formula(0, 0, Formula::new("=Data!C8").set_result("150")).unwrap();
    report.write_formula(1, 0, Formula::new("=SUM(Data!B2:B6)").set_result("150")).unwrap();
    workbook.define_name("Prices", "=Data!$B$2:$B$6").unwrap();
    workbook.save_to_buffer().unwrap()
}

fn open(bytes: Vec<u8>) -> (Workbook, Document, Document) {
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let data = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    let report = workbook.read_sheet(1, &DisplayLocale::default()).unwrap().document;
    (workbook, data, report)
}

#[test]
fn inserted_rows_move_every_reference() {
    let (workbook, mut data, mut report) = open(book());
    assert!(data.sheet_rows_fixed() && !data.sheet_structure_locked());
    // Two rows before row 4.
    let changed = workbook.insert(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Rows, 3, 2).unwrap();
    assert_eq!(changed, [0, 1]);
    assert_eq!(data.row_count(), 10);
    assert_eq!(data.formula(2, 2).as_deref(), Some("B3*2"));
    assert_eq!(data.formula(5, 2).as_deref(), Some("B6*2"));
    assert_eq!(data.formula(9, 2).as_deref(), Some("SUM(B2:B8)"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!C10"));
    assert_eq!(report.formula(1, 0).as_deref(), Some("SUM(Data!B2:B8)"));
    workbook.recalculate(&mut [(0, &mut data), (1, &mut report)], NOW).unwrap();
    assert_eq!(report.cell(0, 0).unwrap(), "150");

    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<mergeCell ref="A10:B10"/>"#), "{sheet}");
    assert!(sheet.contains(r#"sqref="B2:B8""#), "{sheet}");
    assert!(sheet.contains(r#"<dimension ref="A1:C10"/>"#), "{sheet}");
    assert!(part(&saved, "xl/workbook.xml").contains("Data!$B$2:$B$8"));
    let (_, reopened, report) = open(saved);
    assert_eq!(reopened.cell(5, 1).unwrap(), "30");
    assert_eq!(reopened.cell(3, 1).unwrap(), "");
    assert_eq!(reopened.formula(9, 2).as_deref(), Some("SUM(B2:B8)"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!C10"));
}

#[test]
fn deleted_rows_shrink_ranges_and_break_references() {
    let (workbook, mut data, mut report) = open(book());
    // Row 4 goes.
    workbook.delete(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Rows, 3, 1).unwrap();
    assert_eq!(data.formula(3, 2).as_deref(), Some("B4*2"));
    assert_eq!(data.formula(6, 2).as_deref(), Some("SUM(B2:B5)"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!C7"));
    report.set_cell(1, 0, "=Data!B4").unwrap();
    workbook.delete(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Rows, 3, 1).unwrap();
    assert_eq!(report.formula(1, 0).as_deref(), Some("Data!#REF!"));
    workbook.recalculate(&mut [(0, &mut data), (1, &mut report)], NOW).unwrap();
    // 10 + 20 + 50 are left.
    assert_eq!(data.cell(5, 2).unwrap(), "80");

    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<mergeCell ref="A6:B6"/>"#), "{sheet}");
    assert!(sheet.contains(r#"sqref="B2:B4""#), "{sheet}");
    assert!(part(&saved, "xl/workbook.xml").contains("Data!$B$2:$B$4"));
}

#[test]
fn columns_inserted_and_deleted_move_cells_widths_and_references() {
    let (workbook, mut data, mut report) = open(book());
    // A column before B.
    workbook.insert(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Columns, 1, 1).unwrap();
    assert_eq!(data.cell(1, 2).unwrap(), "10");
    assert_eq!(data.formula(1, 3).as_deref(), Some("C2*2"));
    assert_eq!(report.formula(1, 0).as_deref(), Some("SUM(Data!C2:C6)"));
    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<c r="C2"><v>10</v></c>"#), "{sheet}");
    assert!(sheet.contains(r#"<mergeCell ref="A8:C8"/>"#), "{sheet}");
    assert!(sheet.contains(r#"sqref="C2:C6""#), "{sheet}");
    assert!(sheet.contains(r#"<col min="4" max="4""#), "{sheet}");
    assert!(part(&saved, "xl/workbook.xml").contains("Data!$C$2:$C$6"));
    let (_, reopened, _) = open(saved);
    assert_eq!(reopened.cell(1, 2).unwrap(), "10");
    assert_eq!(reopened.formula(7, 3).as_deref(), Some("SUM(C2:C6)"));

    // And the price column goes: references to it break.
    let (workbook, mut data, mut report) = open(book());
    workbook.delete(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Columns, 1, 1).unwrap();
    assert_eq!(data.formula(1, 1).as_deref(), Some("#REF!*2"));
    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("<mergeCell"), "{sheet}");
    assert!(!sheet.contains("conditionalFormatting"), "{sheet}");
    assert!(!sheet.contains("dataValidation"), "{sheet}");
}

#[test]
fn undoing_brings_formulas_back() {
    let (workbook, mut data, mut report) = open(book());
    workbook.insert(&mut [(0, &mut data), (1, &mut report)], 0, Axis::Rows, 1, 3).unwrap();
    data.undo();
    report.undo();
    assert_eq!(data.row_count(), 8);
    assert_eq!(data.formula(7, 2).as_deref(), Some("SUM(B2:B6)"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!C8"));
    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    assert!(part(&saved, "xl/worksheets/sheet1.xml").contains(r#"<mergeCell ref="A8:B8"/>"#));
    let (_, reopened, report) = open(saved);
    assert_eq!(reopened.formula(2, 2).as_deref(), Some("B3*2"));
    assert_eq!(report.formula(1, 0).as_deref(), Some("SUM(Data!B2:B6)"));
}

#[test]
fn tables_keep_rows_in_place() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    for row in 0..4u32 {
        sheet.write_number(row, 0, f64::from(row)).unwrap();
        sheet.write_number(row, 1, f64::from(row)).unwrap();
    }
    sheet.add_table(0, 0, 3, 1, &Table::new()).unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert!(document.sheet_structure_locked());
    assert!(matches!(
        workbook.insert(&mut [(0, &mut document)], 0, Axis::Rows, 1, 1),
        Err(Error::SheetStructure(SheetStructure::RowsFixed))
    ));
}

#[test]
fn shared_formulas_are_written_on_their_own_when_rows_move() {
    // D2:D4 share B2*2, as Excel writes a formula filled down.
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="B1"><v>1</v></c><c r="D1"><f>SUM(D2:D4)</f><v>12</v></c></row>
<row r="2"><c r="B2"><v>1</v></c><c r="D2"><f t="shared" ref="D2:D4" si="0">B2*2</f><v>2</v></c></row>
<row r="3"><c r="B3"><v>2</v></c><c r="D3"><f t="shared" si="0"/><v>4</v></c></row>
<row r="4"><c r="B4"><v>3</v></c><c r="D4"><f t="shared" si="0"/><v>6</v></c></row>
</sheetData></worksheet>"#;
    let relationships = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in [
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{relationships}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)),
        ("xl/workbook.xml", format!(r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{relationships}"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#)),
        ("xl/_rels/workbook.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{relationships}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#)),
        ("xl/worksheets/sheet1.xml", sheet.to_owned()),
    ] {
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    let workbook = Workbook::from_bytes(writer.finish().unwrap().into_inner()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    // A row before row 3.
    workbook.insert(&mut [(0, &mut document)], 0, Axis::Rows, 2, 1).unwrap();
    let saved = workbook.to_bytes(&[(0, &document)]).unwrap();
    let xml = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(!xml.contains("shared"), "{xml}");
    assert!(xml.contains("<f>SUM(D2:D5)</f>"), "{xml}");
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert_eq!(reopened.formula(1, 3).as_deref(), Some("B2*2"));
    assert_eq!(reopened.formula(3, 3).as_deref(), Some("B4*2"));
    assert_eq!(reopened.formula(4, 3).as_deref(), Some("B5*2"));
    assert_eq!(reopened.cell(4, 1).unwrap(), "3");
}
