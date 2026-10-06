use comma_core::{CellRange, Document, FilterOp, Filter, ViewSpec, parse_pasted_text};

fn open(bytes: &[u8]) -> Document {
    Document::from_bytes(bytes.to_vec())
}

fn range(top: usize, left: usize, bottom: usize, right: usize) -> CellRange {
    CellRange { top, left, bottom, right }
}

const TABLE: &[u8] = b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n";

#[test]
fn copies_ranges_as_tsv() {
    let document = open(b"h1,h2\n\"x\ty\",plain\n\"multi\nline\",\"q\"\"\"\n");
    assert_eq!(
        document.copy_range(range(0, 0, 1, 1)).unwrap(),
        "\"x\ty\"\tplain\n\"multi\nline\"\t\"q\"\"\"\n"
    );
}

#[test]
fn parses_pasted_text() {
    assert_eq!(parse_pasted_text("a\tb\r\nc\td\r\n"), [["a", "b"], ["c", "d"]]);
    assert_eq!(parse_pasted_text("one, two\nthree"), [["one, two"], ["three"]]);
    assert_eq!(parse_pasted_text("\"line 1\nline 2\""), [["line 1\nline 2"]]);
    assert_eq!(parse_pasted_text("x\t\"a\tb\"\n"), [["x", "a\tb"]]);
    assert!(parse_pasted_text("").is_empty());
}

#[test]
fn copy_then_paste_round_trips() {
    let source = open(b"h\n\"tab\there\"\n\"new\nline\"\n");
    let text = source.copy_range(range(0, 0, 1, 0)).unwrap();
    let grid = parse_pasted_text(&text);
    assert_eq!(grid, [["tab\there"], ["new\nline"]]);
}

#[test]
fn paste_is_one_step_and_grows_the_table() {
    let mut document = open(TABLE);
    let grid = parse_pasted_text("x\ty\nz\tw\nend\t\n");
    let written = document.paste(2, 2, &grid).unwrap();
    assert_eq!(written, Some(range(2, 2, 4, 3)));
    assert_eq!(document.column_count(), 4);
    assert_eq!(
        document.to_bytes().unwrap(),
        b"a,b,c\n1,2,3\n4,5,6\n7,8,x,y\n,,z,w\n,,end\n"
    );
    document.undo();
    assert_eq!(document.to_bytes().unwrap(), TABLE);
    assert_eq!(document.column_count(), 3);
}

#[test]
fn paste_stops_at_the_last_row_of_a_view() {
    let mut document = open(TABLE);
    document
        .set_view(ViewSpec {
            sort: vec![],
            filters: vec![Filter { column: Some(0), op: FilterOp::GreaterThan, value: "3".into(), case_sensitive: false }],
        })
        .unwrap();
    let written = document.paste(1, 0, &parse_pasted_text("A\nB\nC")).unwrap();
    assert_eq!(written, Some(range(1, 0, 1, 0)));
    assert_eq!(document.to_bytes().unwrap(), b"a,b,c\n1,2,3\n4,5,6\nA,8,9\n");
}

#[test]
fn fill_clear_and_fill_down() {
    let mut document = open(TABLE);
    assert_eq!(document.fill_range(range(0, 1, 1, 2), "").unwrap(), 4);
    assert_eq!(document.to_bytes().unwrap(), b"a,b,c\n1,,\n4,,\n7,8,9\n");
    assert_eq!(document.fill_down(range(0, 0, 2, 0)).unwrap(), 2);
    assert_eq!(document.to_bytes().unwrap(), b"a,b,c\n1,,\n1,,\n1,8,9\n");
    document.undo();
    document.undo();
    assert_eq!(document.to_bytes().unwrap(), TABLE);
    assert!(document.fill_range(range(0, 0, 5, 0), "x").is_err());
}

#[test]
fn range_statistics() {
    let document = open(b"n,t\n1 000,5\n2.5,x\n,\n");
    let stats = document.range_stats(range(0, 0, 2, 1)).unwrap();
    assert_eq!((stats.cells, stats.filled, stats.numbers), (6, 4, 3));
    assert_eq!(stats.sum, 1007.5);
}
