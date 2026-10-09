//! Formulas entered and computed again in workbooks.

use std::io::{Cursor, Read};

use comma_core::{DisplayLocale, Document, Error, Workbook};
use rust_xlsxwriter::{Format, Formula};

fn russian() -> DisplayLocale {
    DisplayLocale {
        decimal_separator: ',',
        group_separator: '\u{a0}',
        short_date: "dd.mm.yyyy".to_owned(),
        ..DisplayLocale::default()
    }
}

/// 2025-01-01 at noon.
const NOW: f64 = 45_658.5;

fn sheet_xml(bytes: &[u8], part: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive.by_name(part).unwrap().read_to_string(&mut text).unwrap();
    text
}

/// Sales with formulas Excel computed: D = B * C, E = D * 2, a total, and a share shown
/// rounded. A second sheet reads the first.
fn book() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let money = Format::new().set_num_format("#,##0.00");
    let percent = Format::new().set_num_format("0.0%");
    let sales = workbook.add_worksheet();
    sales.set_name("Sales").unwrap();
    for (column, name) in ["Item", "Price", "Qty", "Sum", "Double", "Share"].iter().enumerate() {
        sales.write_string(0, column as u16, *name).unwrap();
    }
    for (index, (item, price, quantity)) in [("Tea", 10.0, 2.0), ("Coffee", 20.0, 1.0), ("Sugar", 5.0, 4.0)].iter().enumerate() {
        let row = index as u32 + 1;
        let excel = row + 1;
        sales.write_string(row, 0, *item).unwrap();
        sales.write_number(row, 1, *price).unwrap();
        sales.write_number(row, 2, *quantity).unwrap();
        let sum = price * quantity;
        sales.write_formula_with_format(row, 3, Formula::new(format!("=B{excel}*C{excel}")).set_result(sum.to_string()), &money).unwrap();
        sales.write_formula(row, 4, Formula::new(format!("=D{excel}*2")).set_result((sum * 2.0).to_string())).unwrap();
    }
    sales.write_number_with_format(1, 5, 0.2501, &percent).unwrap();
    sales.write_formula_with_format(4, 3, Formula::new("=SUM(D2:D4)").set_result("60"), &money).unwrap();
    let summary = workbook.add_worksheet();
    summary.set_name("Summary").unwrap();
    summary.write_string(0, 0, "Total").unwrap();
    summary.write_formula(0, 1, Formula::new("=Sales!D5*1").set_result("60")).unwrap();
    workbook.save_to_buffer().unwrap()
}

/// The first sheet, with room for formulas: rows and columns added at the end.
fn open(bytes: Vec<u8>) -> (Workbook, Document) {
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let mut document = workbook.read_sheet(0, &russian()).unwrap().document;
    grow(&mut document, 8, 8);
    (workbook, document)
}

fn grow(document: &mut Document, rows: usize, columns: usize) {
    if document.row_count() < rows {
        document.insert_rows(document.row_count(), rows - document.row_count()).unwrap();
    }
    while document.column_count() < columns {
        document.insert_column(document.column_count()).unwrap();
    }
}

#[test]
fn entered_formulas_are_computed_in_the_cell_format() {
    let (workbook, mut sales) = open(book());
    sales.set_cell(1, 6, "=СУММ(B2:B4;1,5)").unwrap();
    assert_eq!(sales.formula(1, 6).as_deref(), Some("SUM(B2:B4,1.5)"));
    let report = workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert_eq!(report.sheets, [0]);
    assert_eq!(sales.cell(1, 6).unwrap(), "36,5");
    // Exact values, not the 25,0% shown.
    sales.set_cell(2, 6, "=F2*10000").unwrap();
    workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert_eq!(sales.cell(2, 6).unwrap(), "2501");
    assert!(matches!(sales.set_cell(3, 6, "=SUM(B2:B4"), Err(Error::Formula(_))));
}

#[test]
fn editing_a_cell_computes_what_depends_on_it() {
    let (workbook, mut sales) = open(book());
    assert_eq!(sales.cell(4, 3).unwrap(), "60,00");
    sales.set_cell(1, 1, "100").unwrap();
    let report = workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert_eq!(report.sheets, [0]);
    // D2, E2 through D2, and the total through D2.
    assert_eq!(sales.cell(1, 3).unwrap(), "200,00");
    assert_eq!(sales.cell(1, 4).unwrap(), "400");
    assert_eq!(sales.cell(4, 3).unwrap(), "240,00");
    // Rows nothing edited reads keep their stored results.
    assert_eq!(sales.cell(2, 4).unwrap(), "40");

    sales.undo();
    workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert_eq!(sales.cell(1, 3).unwrap(), "20,00");
    assert_eq!(sales.cell(4, 3).unwrap(), "60,00");
}

#[test]
fn formulas_read_other_sheets_loaded_or_not() {
    let workbook = Workbook::from_bytes(book()).unwrap();
    let mut sales = workbook.read_sheet(0, &russian()).unwrap().document;
    let mut summary = workbook.read_sheet(1, &russian()).unwrap().document;
    sales.set_cell(1, 2, "3").unwrap();
    let report = workbook.recalculate(&mut [(0, &mut sales), (1, &mut summary)], NOW).unwrap();
    assert_eq!(report.sheets, [0, 1]);
    assert_eq!(summary.cell(0, 1).unwrap(), "70");

    // A formula on the summary reading a sheet not loaded gets the values the file stores.
    let (workbook, _) = open(book());
    let mut summary = workbook.read_sheet(1, &russian()).unwrap().document;
    grow(&mut summary, 2, 2);
    summary.set_cell(1, 1, "=Sales!B2+Sales!C4").unwrap();
    workbook.recalculate(&mut [(1, &mut summary)], NOW).unwrap();
    assert_eq!(summary.cell(1, 1).unwrap(), "14");
}

#[test]
fn circular_and_unknown_formulas() {
    let (workbook, mut sales) = open(book());
    sales.set_cell(6, 0, "=B7+1").unwrap();
    sales.set_cell(6, 1, "=A7+1").unwrap();
    sales.set_cell(6, 2, "=NEWFUNCTION(1)").unwrap();
    sales.set_cell(6, 3, "=TODAY()").unwrap();
    let report = workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert!(report.circular > 0);
    assert_eq!(report.not_computed, 1);
    assert_eq!(sales.cell(6, 2).unwrap(), "");
    assert_eq!(sales.cell(6, 3).unwrap(), "45658");
}

#[test]
fn entering_a_formula_fixes_rows_of_every_sheet() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    workbook.add_worksheet().write_number(0, 0, 1).unwrap();
    workbook.add_worksheet().write_number(0, 0, 2).unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut first = workbook.read_sheet(0, &russian()).unwrap().document;
    let second = workbook.read_sheet(1, &russian()).unwrap().document;
    assert!(!second.sheet_rows_fixed());
    grow(&mut first, 1, 2);
    first.set_cell(0, 1, "=Sheet2!A1*2").unwrap();
    assert!(second.sheet_rows_fixed());
    assert!(first.sheet_rows_fixed());
}

#[test]
fn saving_writes_new_formulas_and_new_results() {
    let (workbook, mut sales) = open(book());
    sales.set_cell(1, 1, "100").unwrap();
    sales.set_cell(5, 3, "=D5/3").unwrap();
    workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let xml = sheet_xml(&saved, "xl/worksheets/sheet1.xml");
    assert!(xml.contains(r#"<c r="D2" s="1"><f>B2*C2</f><v>200</v></c>"#), "{xml}");
    assert!(xml.contains(r#"<c r="D6"><f>D5/3</f><v>80</v></c>"#), "{xml}");
    // Rows nothing changed stay as they were.
    let original = sheet_xml(&book(), "xl/worksheets/sheet1.xml");
    let row = |xml: &str, number: u32| {
        let start = xml.find(&format!("<row r=\"{number}\"")).unwrap();
        xml[start..start + xml[start..].find("</row>").unwrap()].to_owned()
    };
    assert_eq!(row(&xml, 3), row(&original, 3));

    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &russian()).unwrap().document;
    assert_eq!(reopened.formula(5, 3).as_deref(), Some("D5/3"));
    assert_eq!(reopened.cell(5, 3).unwrap(), "80");
    assert_eq!(reopened.cell(4, 3).unwrap(), "240,00");
}

#[test]
fn long_chains_compute() {
    // A running total down 20 000 rows: each row reads the one above.
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    for row in 0..20_000u32 {
        sheet.write_number(row, 0, 1).unwrap();
        let formula = if row == 0 { "=A1".to_owned() } else { format!("=B{row}+A{}", row + 1) };
        sheet.write_formula(row, 1, Formula::new(formula).set_result((row + 1).to_string())).unwrap();
    }
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    document.set_cell(0, 0, "5").unwrap();
    let started = std::time::Instant::now();
    workbook.recalculate(&mut [(0, &mut document)], NOW).unwrap();
    assert!(started.elapsed().as_secs_f64() < 5.0);
    assert_eq!(document.cell(19_999, 1).unwrap(), "20004");
}

#[test]
fn filling_down_moves_references_and_pasting_enters_formulas() {
    let (workbook, mut sales) = open(book());
    sales.set_cell(1, 6, "=B2+C2").unwrap();
    // A sheet shows every row, the first included: visible rows are rows of the sheet.
    sales.fill_down(comma_core::CellRange { top: 1, left: 6, bottom: 3, right: 6 }).unwrap();
    assert_eq!(sales.formula(2, 6).as_deref(), Some("B3+C3"));
    assert_eq!(sales.formula(3, 6).as_deref(), Some("B4+C4"));
    sales.paste(6, 7, &[vec!["=SUM(G2:G4)".to_owned()], vec!["=H7*2".to_owned()], vec!["text".to_owned()]]).unwrap();
    workbook.recalculate(&mut [(0, &mut sales)], NOW).unwrap();
    assert_eq!(sales.cell(2, 6).unwrap(), "21");
    assert_eq!(sales.cell(6, 7).unwrap(), "42");
    assert_eq!(sales.cell(7, 7).unwrap(), "84");
    assert_eq!(sales.formula(8, 7), None);
}
