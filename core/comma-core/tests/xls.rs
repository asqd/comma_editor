//! Excel 97–2003 workbooks; `fixtures/sales.xls` is written by `fixtures/make_xls.py`.

use comma_core::{DisplayLocale, Document, Error, SheetKind, SheetRange, SheetVisibility, Workbook, WorkbookError, WorkbookFormat};

const SALES: &[u8] = include_bytes!("fixtures/sales.xls");

fn russian() -> DisplayLocale {
    DisplayLocale {
        decimal_separator: ',',
        group_separator: '\u{a0}',
        short_date: "dd.mm.yyyy".to_owned(),
        ..DisplayLocale::default()
    }
}

fn rows(document: &Document) -> Vec<Vec<String>> {
    (0..document.row_count()).map(|row| document.row(row).unwrap()).collect()
}

#[test]
fn lists_the_sheets() {
    let workbook = Workbook::from_bytes(SALES.to_vec()).unwrap();
    assert_eq!(workbook.format(), WorkbookFormat::Xls);
    let sheets: Vec<_> = workbook.sheets().iter().map(|sheet| (sheet.name.as_str(), sheet.visibility, sheet.kind)).collect();
    assert_eq!(
        sheets,
        [
            ("Продажи", SheetVisibility::Visible, SheetKind::Worksheet),
            ("Служебный", SheetVisibility::Hidden, SheetKind::Worksheet),
            ("Заметки", SheetVisibility::Visible, SheetKind::Worksheet),
        ]
    );
    assert_eq!(workbook.active_sheet(), 0);
}

#[test]
fn shows_values_in_their_number_formats() {
    let workbook = Workbook::from_bytes(SALES.to_vec()).unwrap();
    let sheet = workbook.read_sheet(0, &russian()).unwrap();
    let document = &sheet.document;
    assert!(!document.has_header());
    let rows = rows(document);
    assert_eq!(rows[0], ["Товар", "Цена", "Кол-во", "Сумма", "Доля", "Дата", "Время", "Есть"]);
    // The formulas were written without results, so their cells are empty; formulas of .xls
    // workbooks are not shown.
    assert_eq!(rows[1], ["Чай", "250,50 ₽", "4", "", "25,0%", "01.01.2025", "18:00", "TRUE"]);
    assert_eq!(rows[2][7], "FALSE");
    assert_eq!(rows[5][0], "Итого");
    assert_eq!(document.formula(1, 3), None);
    // A string longer than one record goes on in CONTINUE records.
    assert_eq!(rows[7][0].chars().count(), "Длинный текст: ".chars().count() + 15_000);
    assert_eq!(sheet.layout.merged, [SheetRange { top: 5, left: 0, bottom: 5, right: 2 }]);
    assert!(sheet.layout.columns.iter().any(|column| column.first == 0 && column.width == Some(20.0)));
}

#[test]
fn reads_hidden_and_sparse_sheets() {
    let workbook = Workbook::from_bytes(SALES.to_vec()).unwrap();
    assert_eq!(rows(&workbook.read_sheet(1, &DisplayLocale::default()).unwrap().document), [["секрет"]]);
    let notes = workbook.read_sheet(2, &DisplayLocale::default()).unwrap().document;
    assert_eq!(notes.cell(2, 1).unwrap(), "Текст начинается с B3");
    assert_eq!(notes.cell(3, 1).unwrap(), "1234567.891");
    assert!(notes.find_problems(10).problems.is_empty());
}

#[test]
fn exports_that_only_look_like_workbooks_say_what_they_are() {
    let problem = |text: &str| match Workbook::from_bytes(text.as_bytes().to_vec()) {
        Err(Error::Workbook(problem)) => problem,
        _ => panic!("opened {text}"),
    };
    assert_eq!(problem("\u{feff}<html>\n<head><meta charset=utf-8></head><table><tr><td>1</td></tr></table>"), WorkbookError::WebPage);
    assert_eq!(problem("<TABLE border=1><TR><TD>1</TD></TR></TABLE>"), WorkbookError::WebPage);
    assert_eq!(
        problem("<?xml version=\"1.0\"?>\n<Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\"></Workbook>"),
        WorkbookError::Xml2003
    );
    assert_eq!(problem("a,b\n1,2\n"), WorkbookError::NotAWorkbook);
}

#[test]
fn compound_files_that_are_not_workbooks_say_so() {
    let mut empty = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    empty.resize(512, 0);
    assert!(matches!(Workbook::from_bytes(empty), Err(Error::Workbook(WorkbookError::NotAWorkbook))));
    let mut cut = SALES.to_vec();
    cut.truncate(4096);
    assert!(Workbook::from_bytes(cut).is_err());
}

#[test]
fn a_sheet_exports_to_xlsx_with_its_values_and_formats() {
    let workbook = Workbook::from_bytes(SALES.to_vec()).unwrap();
    let document = workbook.read_sheet(0, &russian()).unwrap().document;
    let bytes = document.export(comma_core::ExportFormat::Xlsx, None, "Продажи", false).unwrap();
    let exported = Workbook::from_bytes(bytes.clone()).unwrap().read_sheet(0, &russian()).unwrap().document;
    // Shown as Excel shows it: no row of column letters on top, the same values.
    assert_eq!(rows(&exported)[..3], rows(&document)[..3]);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut sheet = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("xl/worksheets/sheet1.xml").unwrap(), &mut sheet).unwrap();
    // Numbers stay numbers, the price in its money format.
    assert!(sheet.contains(r#"<c r="B2" s="1"><v>250.5</v></c>"#), "{sheet}");
    assert!(!sheet.contains("<pane"), "{sheet}");
}
