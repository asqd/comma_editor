use std::io::{Cursor, Write};

use comma_core::{
    ColumnLayout, DisplayLocale, Document, Error, ExportFormat, SheetKind, SheetRange, SheetVisibility, Workbook,
};
use rust_xlsxwriter::{Format, Formula};

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

/// A workbook as Excel writes it: shared strings, styles with number formats, formulas with
/// their results, merged cells, column widths, frozen panes, a hidden sheet.
fn excel_workbook() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let money = Format::new().set_num_format("#,##0.00\\ \"₽\"");
    let percent = Format::new().set_num_format("0%");
    let date = Format::new().set_num_format("dd.mm.yyyy");
    let sheet = workbook.add_worksheet();
    sheet.set_name("Продажи").unwrap();
    for (column, name) in ["Товар", "Цена", "Кол-во", "Сумма", "Скидка", "Дата", "Есть"].iter().enumerate() {
        sheet.write_string(0, column as u16, *name).unwrap();
    }
    sheet.write_string(1, 0, "Чай").unwrap();
    sheet.write_number_with_format(1, 1, 1250.5, &money).unwrap();
    sheet.write_number(1, 2, 2).unwrap();
    sheet.write_formula(1, 3, Formula::new("=B2*C2").set_result("2501")).unwrap();
    sheet.write_number_with_format(1, 4, 0.15, &percent).unwrap();
    sheet.write_number_with_format(1, 5, 45658.0, &date).unwrap();
    sheet.write_boolean(1, 6, true).unwrap();
    sheet.merge_range(3, 0, 3, 3, "Итого", &Format::new()).unwrap();
    sheet.write_formula(3, 4, Formula::new("=SUM(D2:D3)").set_result("2501")).unwrap();
    sheet.set_column_width(0, 30).unwrap();
    sheet.set_column_hidden(6).unwrap();
    sheet.set_freeze_panes(1, 0).unwrap();
    let notes = workbook.add_worksheet();
    notes.set_name("Заметки").unwrap();
    notes.write_string(0, 0, "скрытый лист").unwrap();
    notes.set_hidden(true);
    workbook.save_to_buffer().unwrap()
}

#[test]
fn reads_a_workbook_written_like_excel() {
    let workbook = Workbook::from_bytes(excel_workbook()).unwrap();
    let names: Vec<_> = workbook.sheets().iter().map(|sheet| (sheet.name.as_str(), sheet.visibility, sheet.kind)).collect();
    assert_eq!(
        names,
        [
            ("Продажи", SheetVisibility::Visible, SheetKind::Worksheet),
            ("Заметки", SheetVisibility::Hidden, SheetKind::Worksheet)
        ]
    );

    let sheet = workbook.read_sheet(0, &russian()).unwrap();
    let document = &sheet.document;
    assert!(document.has_header());
    assert_eq!(document.column_count(), 7);
    assert_eq!(
        rows(document),
        [
            vec!["Товар", "Цена", "Кол-во", "Сумма", "Скидка", "Дата", "Есть"],
            vec!["Чай", "1\u{a0}250,50 ₽", "2", "2501", "15%", "01.01.2025", "TRUE"],
            vec![],
            vec!["Итого", "", "", "", "2501"],
        ]
    );
    assert_eq!(document.formula(1, 3).as_deref(), Some("B2*C2"));
    assert_eq!(document.formula(3, 4).as_deref(), Some("SUM(D2:D3)"));
    assert_eq!(document.formula(1, 1), None);

    let layout = &sheet.layout;
    assert_eq!(layout.merged, [SheetRange { top: 3, left: 0, bottom: 3, right: 3 }]);
    assert_eq!((layout.frozen_rows, layout.frozen_columns), (1, 0));
    assert!(layout.columns.iter().any(|column| column.first == 0 && column.width.is_some_and(|width| width > 29.0)));
    assert!(layout.columns.iter().any(|column| matches!(column, ColumnLayout { first: 6, last: 6, hidden: true, .. })));

    let notes = workbook.read_sheet(1, &DisplayLocale::default()).unwrap();
    assert_eq!(rows(&notes.document), [["скрытый лист"]]);
}

#[test]
fn opens_at_the_tab_shown_when_saved() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    workbook.add_worksheet().write_string(0, 0, "first").unwrap();
    workbook.add_worksheet().write_string(0, 0, "second").unwrap().set_active(true);
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    assert_eq!(workbook.active_sheet(), 1);
    assert_eq!(Workbook::from_bytes(excel_workbook()).unwrap().active_sheet(), 0);
}

#[test]
fn rows_of_a_sheet_are_not_broken_rows() {
    let workbook = Workbook::from_bytes(excel_workbook()).unwrap();
    let document = workbook.read_sheet(0, &russian()).unwrap().document;
    assert!(document.is_sheet());
    assert!(document.find_problems(10).problems.is_empty());
    assert_eq!(document.formula_columns(1), [3]);
    assert_eq!(document.formula_columns(3), [4]);
    assert!(document.formula_columns(0).is_empty());
    assert!(!Document::from_bytes(b"a,b\n1\n".to_vec()).is_sheet());
}

#[test]
fn sorting_and_editing_keep_formulas_with_their_cells() {
    let workbook = Workbook::from_bytes(excel_workbook()).unwrap();
    let mut document = workbook.read_sheet(0, &russian()).unwrap().document;
    document.move_rows(&[3], 1).unwrap();
    assert_eq!(document.formula(1, 4).as_deref(), Some("SUM(D2:D3)"));
    assert!(document.set_cell(1, 4, "100").unwrap());
    assert_eq!(document.formula(1, 4), None);
    document.undo();
    assert_eq!(document.formula(1, 4).as_deref(), Some("SUM(D2:D3)"));
    assert_eq!(document.cell(1, 4).unwrap(), "2501");
}

#[test]
fn values_of_a_csv_exported_to_xlsx_read_back() {
    let csv = "id,name,code,amount\n1,Анна,007,10.5\n2,\"Line\nbreak\",012,-3\n3,\"a \"\"quote\"\"\",100,\n";
    let original = Document::from_bytes(csv.as_bytes().to_vec());
    let bytes = original.export(ExportFormat::Xlsx, None, "data", false).unwrap();
    let sheet = Workbook::from_bytes(bytes).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap();
    assert_eq!(sheet.name, "data");
    // A sheet keeps no empty cells after the last value of a row, so compare cell by cell.
    let cells = |document: &Document| -> Vec<String> {
        (0..document.row_count())
            .flat_map(|row| (0..document.column_count()).map(move |column| (row, column)))
            .map(|(row, column)| document.cell(row, column).unwrap())
            .collect()
    };
    assert_eq!(cells(&sheet.document), cells(&original));
    assert!(sheet.document.has_header());
}

// MARK: Packages built by hand, for what writers do differently

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in parts {
        writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A workbook of one sheet with the given cells, and shared strings and styles if given.
fn package(workbook_pr: &str, sheet_data: &str, strings: Option<&str>, styles: Option<&str>) -> Vec<u8> {
    let root_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIPS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    );
    let workbook = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">{workbook_pr}<sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="2" r:id="rId4"/></sheets></workbook>"#
    );
    // Absolute targets, as some writers use.
    let workbook_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId1" Type="{RELATIONSHIPS}/worksheet" Target="/xl/worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Type="{RELATIONSHIPS}/sharedStrings" Target="sharedStrings.xml"/>
            <Relationship Id="rId3" Type="{RELATIONSHIPS}/styles" Target="styles.xml"/>
            <Relationship Id="rId4" Type="{RELATIONSHIPS}/chartsheet" Target="chartsheets/sheet1.xml"/>
        </Relationships>"#
    );
    let sheet = format!(r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="{MAIN}"><sheetData>{sheet_data}</sheetData></worksheet>"#);
    let strings = strings.map(|strings| format!(r#"<sst xmlns="{MAIN}">{strings}</sst>"#));
    let styles = styles.map(|styles| format!(r#"<styleSheet xmlns="{MAIN}">{styles}</styleSheet>"#));
    let mut parts = vec![
        ("_rels/.rels", root_rels.as_str()),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
    ];
    if let Some(strings) = &strings {
        parts.push(("xl/sharedStrings.xml", strings));
    }
    if let Some(styles) = &styles {
        parts.push(("xl/styles.xml", styles));
    }
    zip(&parts)
}

fn read(bytes: Vec<u8>) -> Document {
    Workbook::from_bytes(bytes).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap().document
}

#[test]
fn shared_formulas_move_to_each_cell() {
    let data = r#"
        <row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="shared" ref="B1:B3" si="0">A1*2</f><v>2</v></c></row>
        <row r="2"><c r="A2"><v>2</v></c><c r="B2"><f t="shared" si="0"/><v>4</v></c></row>
        <row r="3"><c r="A3"><v>3</v></c><c r="B3"><f t="shared" si="0"/><v>6</v></c>
            <c r="C3"><f t="array" ref="C3:C4">A1:A2*10</f><v>10</v></c></row>
        <row r="4"><c r="C4"><v>20</v></c></row>"#;
    let document = read(package("", data, None, None));
    assert_eq!(rows(&document), [vec!["1", "2"], vec!["2", "4"], vec!["3", "6", "10"], vec!["", "", "20"]]);
    assert_eq!(document.formula(0, 1).as_deref(), Some("A1*2"));
    assert_eq!(document.formula(1, 1).as_deref(), Some("A2*2"));
    assert_eq!(document.formula(2, 1).as_deref(), Some("A3*2"));
    assert_eq!(document.formula(2, 2).as_deref(), Some("A1:A2*10"));
    assert_eq!(document.formula(3, 2), None);
}

#[test]
fn every_kind_of_cell() {
    let data = r#"
        <row r="1">
            <c r="A1" t="s"><v>1</v></c>
            <c r="B1" t="inlineStr"><is><t xml:space="preserve"> inline </t></is></c>
            <c r="C1" t="str"><f>"a"&amp;"b"</f><v>ab</v></c>
            <c r="D1" t="b"><v>0</v></c>
            <c r="E1" t="e"><v>#DIV/0!</v></c>
            <c r="F1" t="d" s="1"><v>2025-01-01T18:00:00</v></c>
            <c r="G1" s="2"><v>0.5</v></c>
            <c r="H1" s="1"/>
            <c r="I1"><f>NOW()</f></c>
        </row>
        <row><c t="s"><v>0</v></c><c><v>1E+20</v></c></row>
        <row r="5"><c r="B5" s="2"><v>-0.25</v></c></row>"#;
    let strings = r#"<si><t>line_x000D_
two</t></si><si><r><t>rich </t></r><r><rPr><b/></rPr><t>text</t></r></si>"#;
    let styles = r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm"/></numFmts>
        <cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="10"/></cellXfs>"#;
    let document = read(package("", data, Some(strings), Some(styles)));
    assert_eq!(
        rows(&document),
        [
            vec!["rich text", " inline ", "ab", "FALSE", "#DIV/0!", "2025-01-01 18:00", "50.00%", "", ""],
            vec!["line\r\ntwo", "1E+20"],
            vec![],
            vec![],
            vec!["", "-25.00%"],
        ]
    );
    assert_eq!(document.formula(0, 2).as_deref(), Some("\"a\"&\"b\""));
    // A formula without a stored result still is a formula.
    assert_eq!(document.formula(0, 8).as_deref(), Some("NOW()"));
    // The style-only cell H1 adds nothing, so the row ends at the formula.
    assert_eq!(document.column_count(), 9);
}

#[test]
fn dates_from_1904() {
    let styles = r#"<cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14"/></cellXfs>"#;
    let data = r#"<row r="1"><c r="A1" s="1"><v>0</v></c><c r="B1" s="1"><v>43465</v></c></row>"#;
    let document = read(package(r#"<workbookPr date1904="1"/>"#, data, None, Some(styles)));
    assert_eq!(rows(&document), [["1/1/1904", "1/1/2023"]]);
}

#[test]
fn a_sheet_starting_further_down_keeps_its_place() {
    let data = r#"<row r="3"><c r="C3" t="inlineStr"><is><t>here</t></is></c></row>"#;
    let document = read(package("", data, None, None));
    assert_eq!(document.row_count(), 3);
    assert_eq!(document.column_count(), 3);
    assert_eq!(document.cell(2, 2).unwrap(), "here");
    assert!(!document.has_header());
}

#[test]
fn an_empty_sheet_is_one_empty_cell() {
    let document = read(package("", "", None, None));
    assert_eq!((document.row_count(), document.column_count()), (1, 1));
}

#[test]
fn strict_workbooks_read_like_transitional_ones() {
    let strict_main = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    let strict_relationships = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let bytes = zip(&[
        (
            "_rels/.rels",
            &format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{strict_relationships}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
        ),
        (
            "xl/workbook.xml",
            &format!(r#"<x:workbook xmlns:x="{strict_main}" xmlns:r="{strict_relationships}"><x:sheets><x:sheet name="S" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            &format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{strict_relationships}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#),
        ),
        (
            "xl/worksheets/sheet1.xml",
            &format!(r#"<x:worksheet xmlns:x="{strict_main}"><x:sheetData><x:row r="1"><x:c r="A1"><x:v>42</x:v></x:c></x:row></x:sheetData></x:worksheet>"#),
        ),
    ]);
    assert_eq!(rows(&read(bytes)), [["42"]]);
}

#[test]
fn files_that_are_not_workbooks_say_why() {
    let message = |result: Result<Workbook, Error>| match result {
        Err(Error::Workbook(message)) => message,
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("opened"),
    };
    assert!(message(Workbook::from_bytes(b"a,b\n1,2\n".to_vec())).contains("not an Excel workbook"));
    let mut compound = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    compound.resize(512, 0);
    assert!(message(Workbook::from_bytes(compound)).contains(".xls"));
    assert!(message(Workbook::from_bytes(zip(&[("hello.txt", "hi")]))).contains("not an Excel workbook"));

    let workbook = Workbook::from_bytes(package("", "", None, None)).unwrap();
    assert_eq!(workbook.sheets()[1].kind, SheetKind::Chartsheet);
    assert!(matches!(workbook.read_sheet(1, &DisplayLocale::default()), Err(Error::Workbook(_))));
    assert!(matches!(workbook.read_sheet(2, &DisplayLocale::default()), Err(Error::Workbook(_))));
}

#[test]
fn damaged_xml_names_the_part() {
    let bytes = package("", r#"<row r="1"><c r="A1"><v>1</v></c></row><!-- never closed"#, None, None);
    match Workbook::from_bytes(bytes).unwrap().read_sheet(0, &DisplayLocale::default()) {
        Err(Error::Workbook(message)) => assert!(message.contains("xl/worksheets/sheet1.xml"), "{message}"),
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("read a damaged sheet"),
    }
}
