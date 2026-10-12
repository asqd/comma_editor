//! Inserting and deleting rows and columns of workbooks, with references moved.

use std::io::{Cursor, Read, Write};

use comma_core::{Axis, DisplayLocale, Document, Error, SheetStructure, Workbook};
use rust_xlsxwriter::{
    ConditionalFormatFormula, DataValidation, DataValidationRule, Format, Formula,
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
fn charts_follow_rows() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    for row in 0..4u32 {
        sheet.write_number(row, 0, f64::from(row)).unwrap();
    }
    let mut chart = rust_xlsxwriter::Chart::new(rust_xlsxwriter::ChartType::Column);
    chart.add_series().set_values("Sheet1!$A$1:$A$4");
    sheet.insert_chart(5, 2, &chart).unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert!(!document.sheet_structure_locked());
    // A row before row 2.
    workbook.insert(&mut [(0, &mut document)], 0, Axis::Rows, 1, 1).unwrap();
    let saved = workbook.to_bytes(&[(0, &document)]).unwrap();
    let chart = part(&saved, "xl/charts/chart1.xml");
    assert!(chart.contains("<c:f>Sheet1!$A$1:$A$5</c:f>"), "{chart}");
    let drawing = part(&saved, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<xdr:row>6</xdr:row>"), "{drawing}");
}

/// The book with a pivot table on Pivot of the items and quantities of Data.
fn pivot_book() -> Vec<u8> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let data = workbook.add_worksheet();
    data.set_name("Data").unwrap();
    data.write_row(0, 0, ["Item", "Qty"]).unwrap();
    for (row, (item, quantity)) in [("Apples", 1.0), ("Pears", 2.0), ("Apples", 3.0), ("Plums", 4.0)].into_iter().enumerate() {
        data.write_string(row as u32 + 1, 0, item).unwrap();
        data.write_number(row as u32 + 1, 1, quantity).unwrap();
    }
    workbook.add_worksheet().set_name("Pivot").unwrap();
    let bytes = workbook.save_to_buffer().unwrap();
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let relationships = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let package = "http://schemas.openxmlformats.org/package/2006/relationships";
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().to_owned();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        let text = match name.as_str() {
            "[Content_Types].xml" => text.replace("</Types>", r#"<Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/><Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/></Types>"#),
            "xl/_rels/workbook.xml.rels" => text.replace("</Relationships>", &format!(r#"<Relationship Id="rId90" Type="{relationships}/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#)),
            "xl/workbook.xml" => text.replace("</workbook>", r#"<pivotCaches><pivotCache cacheId="1" r:id="rId90"/></pivotCaches></workbook>"#),
            _ => text,
        };
        writer.start_file(name, options).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    for (name, text) in [
        ("xl/worksheets/_rels/sheet2.xml.rels", format!(r#"<Relationships xmlns="{package}"><Relationship Id="rId1" Type="{relationships}/pivotTable" Target="../pivotTables/pivotTable1.xml"/></Relationships>"#)),
        ("xl/pivotCache/pivotCacheDefinition1.xml", format!(r#"<pivotCacheDefinition xmlns="{main}" refreshOnLoad="1" saveData="0" recordCount="4"><cacheSource type="worksheet"><worksheetSource ref="A1:B5" sheet="Data"/></cacheSource><cacheFields count="2"><cacheField name="Item" numFmtId="0"><sharedItems count="3"><s v="Apples"/><s v="Pears"/><s v="Plums"/></sharedItems></cacheField><cacheField name="Qty" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="1" maxValue="4"/></cacheField></cacheFields></pivotCacheDefinition>"#)),
        ("xl/pivotTables/pivotTable1.xml", format!(r#"<pivotTableDefinition xmlns="{main}" name="P1" cacheId="1" dataCaption="Values"><location ref="A3:B7" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/><pivotFields count="2"><pivotField axis="axisRow" showAll="0"><items count="4"><item x="0"/><item x="1"/><item x="2"/><item t="default"/></items></pivotField><pivotField dataField="1" showAll="0"/></pivotFields><rowFields count="1"><field x="0"/></rowFields><dataFields count="1"><dataField name="Sum of Qty" fld="1"/></dataFields></pivotTableDefinition>"#)),
        ("xl/pivotTables/_rels/pivotTable1.xml.rels", format!(r#"<Relationships xmlns="{package}"><Relationship Id="rId1" Type="{relationships}/pivotCacheDefinition" Target="../pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#)),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn pivot_tables_read_the_rows_they_read() {
    let (workbook, mut data, pivot) = open(pivot_book());
    assert!(!data.sheet_structure_locked());
    // The pivot table lays itself out on its sheet, which keeps its rows.
    let mut pivot = pivot;
    assert!(matches!(workbook.insert(&mut [(1, &mut pivot)], 1, Axis::Rows, 0, 1), Err(Error::SheetStructure(SheetStructure::PivotTable))));
    // A row before row 3 of the source, one after it.
    workbook.insert(&mut [(0, &mut data)], 0, Axis::Rows, 2, 1).unwrap();
    workbook.insert(&mut [(0, &mut data)], 0, Axis::Rows, 7, 1).unwrap();
    let saved = workbook.to_bytes(&[(0, &data)]).unwrap();
    let cache = part(&saved, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(cache.contains(r#"<worksheetSource ref="A1:B6" sheet="Data"/>"#), "{cache}");
}

#[test]
fn comments_and_shapes_follow_rows() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    for row in 0..10u32 {
        sheet.write_number(row, 0, f64::from(row)).unwrap();
    }
    // Notes on B3 and B6, a text box from D2 down to G8.
    sheet.insert_note(2, 1, &rust_xlsxwriter::Note::new("Third")).unwrap();
    sheet.insert_note(5, 1, &rust_xlsxwriter::Note::new("Sixth")).unwrap();
    sheet.insert_shape(1, 3, &rust_xlsxwriter::Shape::textbox().set_text("Box")).unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert!(!document.sheet_structure_locked());
    // Two rows before row 2, then row 8 (the sixth, as it was) goes; a column before A, so
    // the box, right of the cells read, moves too.
    workbook.insert(&mut [(0, &mut document)], 0, Axis::Rows, 1, 2).unwrap();
    workbook.delete(&mut [(0, &mut document)], 0, Axis::Rows, 7, 1).unwrap();
    workbook.insert(&mut [(0, &mut document)], 0, Axis::Columns, 0, 1).unwrap();
    let saved = workbook.to_bytes(&[(0, &document)]).unwrap();
    let comments = part(&saved, "xl/comments1.xml");
    assert!(comments.contains(r#"ref="C5""#) && !comments.contains("Sixth"), "{comments}");
    let notes = part(&saved, "xl/drawings/vmlDrawing1.vml");
    assert!(notes.contains("<x:Row>4</x:Row><x:Column>2</x:Column>") && notes.matches("<v:shape ").count() == 1, "{notes}");
    let drawing = part(&saved, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<xdr:from><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row>"), "{drawing}");
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert_eq!(reopened.row_count(), 11);
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

#[test]
fn moved_rows_take_references_with_them() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sales = workbook.add_worksheet();
    sales.set_name("Sales").unwrap();
    sales.write_row(0, 0, ["Item", "Price", "Qty", "Sum"]).unwrap();
    for (index, (item, price, quantity)) in [("Tea", 10.0, 2.0), ("Coffee", 20.0, 1.0), ("Sugar", 5.0, 4.0)].iter().enumerate() {
        let row = index as u32 + 1;
        sales.write_string(row, 0, *item).unwrap();
        sales.write_number(row, 1, *price).unwrap();
        sales.write_number(row, 2, *quantity).unwrap();
        sales.write_formula(row, 3, format!("=B{0}*C{0}", row + 1).as_str()).unwrap();
    }
    sales.write_formula(4, 3, "=SUM(D2:D4)").unwrap();
    let summary = workbook.add_worksheet();
    summary.set_name("Summary").unwrap();
    summary.write_formula(0, 0, "=Sales!D2").unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut sales = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    let mut summary = workbook.read_sheet(1, &DisplayLocale::default()).unwrap().document;
    assert!(sales.sheet_rows_fixed());
    // Tea goes after Sugar.
    let changed = workbook.move_rows(&mut [(0, &mut sales), (1, &mut summary)], 0, &[1], 4).unwrap();
    assert_eq!(changed, vec![0, 1]);
    assert_eq!((1..4).map(|row| sales.cell(row, 0).unwrap()).collect::<Vec<_>>(), ["Coffee", "Sugar", "Tea"]);
    assert_eq!(sales.formula(3, 3).as_deref(), Some("B4*C4"));
    // The sum keeps its rows; the summary follows Tea.
    assert_eq!(sales.formula(4, 3).as_deref(), Some("SUM(D2:D4)"));
    assert_eq!(summary.formula(0, 0).as_deref(), Some("Sales!D4"));
    workbook.recalculate(&mut [(0, &mut sales), (1, &mut summary)], 45658.0).unwrap();
    assert_eq!(summary.cell(0, 0).unwrap(), "20");
    let saved = workbook.to_bytes(&[(0, &sales), (1, &summary)]).unwrap();
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert_eq!(reopened.cell(3, 0).unwrap(), "Tea");
    assert_eq!(reopened.formula(3, 3).as_deref(), Some("B4*C4"));
    // One undo puts the rows back.
    assert!(sales.undo());
    assert_eq!(sales.cell(1, 0).unwrap(), "Tea");
}

#[test]
fn sorting_a_sheet_with_formulas_works_as_excel_sorts() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let money = rust_xlsxwriter::Format::new().set_num_format("#,##0.00");
    let sales = workbook.add_worksheet();
    sales.set_name("Sales").unwrap();
    sales.write_row(0, 0, ["Item", "Price", "Qty", "Sum"]).unwrap();
    for (index, (item, price, quantity)) in [("Tea", 10.004, 2.0), ("Coffee", 20.0, 1.0), ("Sugar", 5.0, 4.0)].iter().enumerate() {
        let row = index as u32 + 1;
        sales.write_string(row, 0, *item).unwrap();
        sales.write_number_with_format(row, 1, *price, &money).unwrap();
        sales.write_number(row, 2, *quantity).unwrap();
        sales.write_formula(row, 3, format!("=B{0}*C{0}", row + 1).as_str()).unwrap();
    }
    // Coffee's quantity has a format of its own, which goes with it.
    sales.write_number_with_format(2, 2, 1.0, &rust_xlsxwriter::Format::new().set_num_format("0.0")).unwrap();
    sales.write_formula(4, 3, "=SUM(D2:D4)").unwrap();
    sales.write_formula(5, 3, "=D2").unwrap();
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut sales = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    sales.set_has_header(true);
    // Sort the three items by name, the total rows outside the sort (they are empty in A).
    sales.set_view(comma_core::ViewSpec { sort: vec![comma_core::SortKey { column: 0, ascending: true }], filters: vec![] }).unwrap();
    assert!(workbook.sort_rows(&mut sales, 0).unwrap());
    let items: Vec<String> = (1..4).map(|row| sales.cell(row, 0).unwrap()).collect();
    assert_eq!(items, ["Coffee", "Sugar", "Tea"]);
    assert_eq!((sales.cell(1, 2).unwrap(), sales.cell(3, 2).unwrap()), ("1.0".to_owned(), "2".to_owned()));
    // A formula moved with its row reads its row; one outside keeps its cells, as in Excel.
    assert_eq!(sales.formula(3, 3).as_deref(), Some("B4*C4"));
    assert_eq!(sales.formula(5, 3).as_deref(), Some("D2"));
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved.clone())).unwrap();
    let mut xml = String::new();
    archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut xml).unwrap();
    // Tea's price moved exactly, not as the 10,00 shown.
    assert!(xml.contains("<v>10.004</v>"), "{xml}");
    let reopened = Workbook::from_bytes(saved).unwrap().read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert_eq!(reopened.cell(3, 0).unwrap(), "Tea");
    // Formats went with the values: Coffee's 1.0 is on the first row, Tea's price on the third.
    assert_eq!((reopened.cell(1, 2).unwrap(), reopened.cell(3, 2).unwrap()), ("1.0".to_owned(), "2".to_owned()));
    assert_eq!(reopened.cell(3, 1).unwrap(), "10.00");
    assert_eq!(reopened.formula(3, 3).as_deref(), Some("B4*C4"));
    assert!(sales.undo());
    assert_eq!(sales.cell(1, 0).unwrap(), "Tea");
}

#[test]
fn sorted_rows_take_their_links_along() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_row(0, 0, ["Item", "Link"]).unwrap();
    for (index, item) in ["b", "c", "a"].iter().enumerate() {
        let row = index as u32 + 1;
        sheet.write_string(row, 0, *item).unwrap();
        sheet.write_url_with_text(row, 1, format!("https://example.com/{item}").as_str(), *item).unwrap();
    }
    let workbook = Workbook::from_bytes(workbook.save_to_buffer().unwrap()).unwrap();
    let mut document = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    document.set_has_header(true);
    document.set_view(comma_core::ViewSpec { sort: vec![comma_core::SortKey { column: 0, ascending: true }], filters: vec![] }).unwrap();
    assert!(workbook.sort_rows(&mut document, 0).unwrap());
    let saved = workbook.to_bytes(&[(0, &document)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved)).unwrap();
    let mut xml = String::new();
    archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut xml).unwrap();
    let mut rels = String::new();
    archive.by_name("xl/worksheets/_rels/sheet1.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
    // "a" was the third row's link: it is on the second row now, with its text.
    let id_of = |url: &str| rels.split("<Relationship ").find(|item| item.contains(url)).and_then(|item| item.split("Id=\"").nth(1)).map(|rest| rest[..rest.find('"').unwrap()].to_owned()).unwrap();
    assert!(xml.contains(&format!(r#"<hyperlink ref="B2" r:id="{}""#, id_of("example.com/a"))), "{xml}");
    assert!(xml.contains(&format!(r#"<hyperlink ref="B4" r:id="{}""#, id_of("example.com/c"))), "{xml}");
}

fn table_book() -> Vec<u8> {
    use rust_xlsxwriter::{Table, TableColumn};
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Sales").unwrap();
    for (row, (item, price, quantity)) in [("Tea", 10.0, 2.0), ("Coffee", 20.0, 1.0), ("Sugar", 5.0, 4.0)].iter().enumerate() {
        let row = row as u32 + 1;
        sheet.write_string(row, 0, *item).unwrap();
        sheet.write_number(row, 1, *price).unwrap();
        sheet.write_number(row, 2, *quantity).unwrap();
    }
    let columns = vec![
        TableColumn::new().set_header("Item"),
        TableColumn::new().set_header("Price"),
        TableColumn::new().set_header("Qty"),
        TableColumn::new().set_header("Total").set_formula("[@Price]*[@Qty]"),
    ];
    sheet.add_table(0, 0, 3, 3, &Table::new().set_name("Orders").set_columns(&columns)).unwrap();
    sheet.write_formula(5, 0, "=SUM(Orders[Total])").unwrap();
    workbook.save_to_buffer().unwrap()
}

#[test]
fn rows_inserted_into_a_table_extend_it_and_fill_its_formulas() {
    let workbook = Workbook::from_bytes(table_book()).unwrap();
    let mut sales = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    assert!(!sales.sheet_structure_locked());
    // A row between Coffee and Sugar, inside the table.
    workbook.insert(&mut [(0, &mut sales)], 0, Axis::Rows, 3, 1).unwrap();
    assert_eq!(sales.formula(3, 3).as_deref(), Some("[[#This Row],Price]*[[#This Row],Qty]"));
    sales.set_cell(3, 0, "Milk").unwrap();
    sales.set_cell(3, 1, "4").unwrap();
    sales.set_cell(3, 2, "5").unwrap();
    workbook.recalculate(&mut [(0, &mut sales)], 45658.0).unwrap();
    assert_eq!(sales.cell(3, 3).unwrap(), "20");
    // The sum reads the whole table, the new row too: 20 + 20 + 20 + 20.
    assert_eq!(sales.cell(6, 0).unwrap(), "80");
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved.clone())).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:D5""#), "{table}");
    let reopened = Workbook::from_bytes(saved).unwrap();
    let mut again = reopened.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    reopened.recalculate(&mut [(0, &mut again)], 45658.0).unwrap();
    assert_eq!(again.cell(3, 0).unwrap(), "Milk");
    assert_eq!(again.cell(6, 0).unwrap(), "80");
    // The header row stays.
    assert!(matches!(workbook.delete(&mut [(0, &mut sales)], 0, Axis::Rows, 0, 1), Err(Error::SheetStructure(SheetStructure::TableEdges))));
    // Rows move among the table's data, not out of it, and its headers stay.
    assert!(matches!(workbook.move_rows(&mut [(0, &mut sales)], 0, &[1], 6), Err(Error::SheetStructure(SheetStructure::TableRowsMoved))));
    assert!(matches!(workbook.move_rows(&mut [(0, &mut sales)], 0, &[0], 3), Err(Error::SheetStructure(SheetStructure::TableRowsMoved))));
    let first = sales.cell(1, 0).unwrap();
    workbook.move_rows(&mut [(0, &mut sales)], 0, &[1], 4).unwrap();
    assert_eq!(sales.cell(3, 0).unwrap(), first);
    assert_eq!(sales.cell(6, 0).unwrap(), "80");
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved)).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:D5""#), "{table}");
    // Deleting a data row shrinks it.
    workbook.delete(&mut [(0, &mut sales)], 0, Axis::Rows, 1, 1).unwrap();
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved)).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:D4""#), "{table}");
}

#[test]
fn rows_of_a_table_sort_among_themselves() {
    // The table alone on its sheet: its data rows sort, the sum below would join them.
    let bytes = table_book();
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let mut sales = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    sales.set_has_header(true);
    let by_item = comma_core::ViewSpec { sort: vec![comma_core::SortKey { column: 0, ascending: true }], filters: vec![] };
    sales.set_view(by_item.clone()).unwrap();
    // The sum's row, empty in A but for its formula, sorts in with the table's rows.
    assert!(matches!(workbook.sort_rows(&mut sales, 0), Err(Error::SheetStructure(SheetStructure::TableRowsMoved))));
    // Without it, the items sort.
    sales.set_view(comma_core::ViewSpec { sort: vec![], filters: vec![] }).unwrap();
    workbook.delete(&mut [(0, &mut sales)], 0, Axis::Rows, 4, 2).unwrap();
    sales.set_view(by_item).unwrap();
    assert!(workbook.sort_rows(&mut sales, 0).unwrap());
    let items: Vec<String> = (1..4).map(|row| sales.cell(row, 0).unwrap()).collect();
    assert_eq!(items, ["Coffee", "Sugar", "Tea"]);
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved.clone())).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:D4""#), "{table}");
    let reopened = Workbook::from_bytes(saved).unwrap();
    let mut again = reopened.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    reopened.recalculate(&mut [(0, &mut again)], 45658.0).unwrap();
    // Coffee 20 × 1 on the first data row.
    assert_eq!((again.cell(1, 0).unwrap(), again.cell(1, 3).unwrap()), ("Coffee".to_owned(), "20".to_owned()));
}

#[test]
fn columns_inserted_into_a_table_join_it() {
    let workbook = Workbook::from_bytes(table_book()).unwrap();
    let mut sales = workbook.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    // A column between Price and Qty: a column of the table, named as Excel names one.
    workbook.insert(&mut [(0, &mut sales)], 0, Axis::Columns, 2, 1).unwrap();
    assert_eq!(sales.cell(0, 2).unwrap(), "Column1");
    sales.set_cell(1, 2, "x").unwrap();
    workbook.recalculate(&mut [(0, &mut sales)], 45658.0).unwrap();
    // The total still reads its row's price and quantity, a column further right.
    assert_eq!(sales.cell(1, 4).unwrap(), "20");
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved.clone())).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:E4""#), "{table}");
    assert!(table.contains(r#"<tableColumns count="5">"#) && table.contains(r#"name="Column1"/>"#), "{table}");
    let names: Vec<&str> = table.match_indices("name=\"").map(|(at, _)| table[at + 6..].split('"').next().unwrap()).collect();
    assert_eq!(names[..6], ["Orders", "Item", "Price", "Column1", "Qty", "Total"]);
    let reopened = Workbook::from_bytes(saved).unwrap();
    let mut again = reopened.read_sheet(0, &DisplayLocale::default()).unwrap().document;
    reopened.recalculate(&mut [(0, &mut again)], 45658.0).unwrap();
    assert_eq!((again.cell(1, 2).unwrap(), again.cell(5, 0).unwrap()), ("x".to_owned(), "60".to_owned()));
    // The new column goes again; Price, which Total reads, and the whole table stay.
    workbook.delete(&mut [(0, &mut sales)], 0, Axis::Columns, 2, 1).unwrap();
    assert_eq!(sales.cell(0, 2).unwrap(), "Qty");
    assert!(matches!(workbook.delete(&mut [(0, &mut sales)], 0, Axis::Columns, 1, 1), Err(Error::SheetStructure(SheetStructure::TableColumnInUse))));
    assert!(matches!(workbook.delete(&mut [(0, &mut sales)], 0, Axis::Columns, 0, 4), Err(Error::SheetStructure(SheetStructure::TableColumns))));
    let saved = workbook.to_bytes(&[(0, &sales)]).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(saved)).unwrap();
    let mut table = String::new();
    archive.by_name("xl/tables/table1.xml").unwrap().read_to_string(&mut table).unwrap();
    assert!(table.contains(r#"ref="A1:D4""#) && table.contains(r#"<tableColumns count="4">"#), "{table}");
}

#[test]
fn split_and_merged_columns_move_references() {
    let (workbook, mut data, mut report) = open(book());
    // "Item 2" into "Item" and "2" right after A: B and C move to D and E.
    let (changed, count) = workbook.split_column(&mut [(0, &mut data), (1, &mut report)], 0, 0, " ", false).unwrap();
    assert_eq!((count, changed), (2, vec![0, 1]));
    assert_eq!((data.cell(2, 1).unwrap(), data.cell(2, 2).unwrap()), ("Item".to_owned(), "2".to_owned()));
    assert_eq!(data.formula(2, 4).as_deref(), Some("D3*2"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!E8"));
    assert!(data.undo());
    report.undo();
    assert_eq!(data.formula(2, 2).as_deref(), Some("B3*2"));
    // A and B joined into a new column after B.
    workbook.merge_columns(&mut [(0, &mut data), (1, &mut report)], 0, &[0, 1], ": ").unwrap();
    assert_eq!(data.cell(2, 2).unwrap(), "Item 2: 20");
    assert_eq!(data.formula(2, 3).as_deref(), Some("B3*2"));
    assert_eq!(report.formula(1, 0).as_deref(), Some("SUM(Data!B2:B6)"));
    let saved = workbook.to_bytes(&[(0, &data), (1, &report)]).unwrap();
    let (_, reopened, report) = open(saved);
    assert_eq!(reopened.cell(2, 2).unwrap(), "Item 2: 20");
    assert_eq!(reopened.formula(7, 3).as_deref(), Some("SUM(B2:B6)"));
    assert_eq!(report.formula(0, 0).as_deref(), Some("Data!D8"));
}
