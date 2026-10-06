use comma_core::{CellRange, Document, ExportFormat, Filter, FilterOp, ViewSpec};

const PEOPLE: &str = "id,name,amount,code\n1,\"Ann \"\"A\"\"\",10.5,007\n2,Bob | B,,012\n3,\"Line\nbreak\",-3,100\n";

fn export(document: &Document, format: ExportFormat) -> String {
    String::from_utf8(document.export(format, None, "people", false).unwrap()).unwrap()
}

fn open(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec())
}

#[test]
fn json_keeps_codes_as_text_and_numbers_as_numbers() {
    assert_eq!(
        export(&open(PEOPLE), ExportFormat::Json),
        "[\n  \
         {\"id\": 1, \"name\": \"Ann \\\"A\\\"\", \"amount\": 10.5, \"code\": \"007\"},\n  \
         {\"id\": 2, \"name\": \"Bob | B\", \"amount\": null, \"code\": \"012\"},\n  \
         {\"id\": 3, \"name\": \"Line\\nbreak\", \"amount\": -3, \"code\": \"100\"}\n]\n"
    );
}

#[test]
fn markdown_escapes_pipes_and_line_breaks() {
    assert_eq!(
        export(&open(PEOPLE), ExportFormat::Markdown),
        "| id | name | amount | code |\n\
         | ---: | --- | ---: | --- |\n\
         | 1 | Ann \"A\" | 10.5 | 007 |\n\
         | 2 | Bob \\| B |  | 012 |\n\
         | 3 | Line<br>break | -3 | 100 |\n"
    );
}

#[test]
fn sql_creates_and_inserts() {
    let sql = export(&open(PEOPLE), ExportFormat::Sql);
    assert!(sql.starts_with(
        "CREATE TABLE \"people\" (\n  \"id\" NUMERIC,\n  \"name\" TEXT,\n  \"amount\" NUMERIC,\n  \"code\" TEXT\n);\n\n"
    ));
    assert!(sql.contains("INSERT INTO \"people\" (\"id\", \"name\", \"amount\", \"code\") VALUES (2, 'Bob | B', NULL, '012');\n"));
}

#[test]
fn html_escapes_and_fragments() {
    let document = open("a,b\n<x>,1\n");
    let page = export(&document, ExportFormat::Html);
    assert!(page.starts_with("<!DOCTYPE html>") && page.contains("<td>&lt;x&gt;</td><td class=\"number\">1</td>"));
    let fragment = String::from_utf8(document.export(ExportFormat::Html, None, "t", true).unwrap()).unwrap();
    assert!(fragment.starts_with("<table>"));
}

#[test]
fn exports_follow_the_view_and_the_range() {
    let mut document = open(PEOPLE);
    document
        .set_view(ViewSpec {
            sort: vec![],
            filters: vec![Filter { column: Some(0), op: FilterOp::GreaterThan, value: "1".into(), case_sensitive: false }],
        })
        .unwrap();
    let range = CellRange { top: 0, left: 1, bottom: 1, right: 2 };
    let sql = String::from_utf8(document.export(ExportFormat::Sql, Some(range), "t", true).unwrap()).unwrap();
    assert_eq!(
        sql,
        "INSERT INTO \"t\" (\"name\", \"amount\") VALUES ('Bob | B', NULL);\n\
         INSERT INTO \"t\" (\"name\", \"amount\") VALUES ('Line\nbreak', -3);\n"
    );
}

#[test]
fn without_a_header_columns_are_letters() {
    let document = open("1,x\n2,y\n");
    assert!(!document.has_header());
    assert!(export(&document, ExportFormat::Json).starts_with("[\n  {\"A\": 1, \"B\": \"x\"}"));
}

#[test]
fn xlsx_is_a_zip_with_a_sheet() {
    let bytes = open("name,sum\nАнна,\"1 250,50\"\n").export(ExportFormat::Xlsx, None, "data", false).unwrap();
    assert_eq!(&bytes[..2], b"PK");
    assert!(bytes.windows(19).any(|w| w == b"xl/worksheets/sheet"));
}
