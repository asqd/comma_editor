use comma_core::{Document, Error, Filter, FilterOp, SearchQuery, SortKey, ViewSpec};

const PEOPLE: &[u8] = b"name,city,amount\n\
Bob,London,10\n\
alice,Moscow,9\n\
Clara,london,100\n\
Dan,,\"1 000,5\"\n";

fn open(bytes: &[u8]) -> Document {
    Document::from_bytes(bytes.to_vec())
}

fn visible(document: &Document, column: usize) -> Vec<String> {
    (0..document.visible_row_count())
        .map(|row| document.visible_row(row).unwrap()[column].clone())
        .collect()
}

fn sort(column: usize, ascending: bool) -> ViewSpec {
    ViewSpec {
        sort: vec![SortKey { column, ascending }],
        filters: vec![],
    }
}

fn filter(column: Option<usize>, op: FilterOp, value: &str) -> Filter {
    Filter {
        column,
        op,
        value: value.to_owned(),
        case_sensitive: false,
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        ..SearchQuery::default()
    }
}

#[test]
fn visible_rows_skip_the_header() {
    let document = open(PEOPLE);
    assert_eq!(document.visible_row_count(), 4);
    assert_eq!(visible(&document, 0), ["Bob", "alice", "Clara", "Dan"]);
    assert_eq!(document.visible_record(0), Some(1));
}

#[test]
fn sorting_does_not_change_the_file() {
    let mut document = open(PEOPLE);
    document.set_view(sort(0, true)).unwrap();
    assert_eq!(visible(&document, 0), ["alice", "Bob", "Clara", "Dan"]);
    assert_eq!(document.visible_index(1), Some(1));
    assert_eq!(document.visible_index(2), Some(0));
    assert_eq!(document.visible_index(0), None);
    document.set_view(sort(2, false)).unwrap();
    assert_eq!(visible(&document, 2), ["1 000,5", "100", "10", "9"]);
    document.set_view(sort(1, true)).unwrap();
    assert_eq!(visible(&document, 1), ["London", "london", "Moscow", ""]);
    assert_eq!(document.to_bytes().unwrap(), PEOPLE);
}

#[test]
fn multi_column_sort() {
    let mut document = open(b"a,b\nx,2\ny,1\nx,1\n");
    document
        .set_view(ViewSpec {
            sort: vec![
                SortKey { column: 0, ascending: true },
                SortKey { column: 1, ascending: true },
            ],
            filters: vec![],
        })
        .unwrap();
    let rows: Vec<Vec<String>> = (0..3).map(|row| document.visible_row(row).unwrap()).collect();
    assert_eq!(rows, [["x", "1"], ["x", "2"], ["y", "1"]]);
}

#[test]
fn filters_combine_with_and() {
    let mut document = open(PEOPLE);
    let spec = |filters| ViewSpec { sort: vec![], filters };
    document
        .set_view(spec(vec![filter(Some(1), FilterOp::Equals, "LONDON")]))
        .unwrap();
    assert_eq!(visible(&document, 0), ["Bob", "Clara"]);

    document
        .set_view(spec(vec![
            filter(Some(1), FilterOp::Equals, "london"),
            filter(Some(2), FilterOp::GreaterThan, "50"),
        ]))
        .unwrap();
    assert_eq!(visible(&document, 0), ["Clara"]);

    document
        .set_view(spec(vec![filter(None, FilterOp::Contains, "mos")]))
        .unwrap();
    assert_eq!(visible(&document, 0), ["alice"]);

    document
        .set_view(spec(vec![filter(None, FilterOp::NotContains, "on")]))
        .unwrap();
    assert_eq!(visible(&document, 0), ["alice", "Dan"]);

    document
        .set_view(spec(vec![filter(Some(1), FilterOp::IsEmpty, "")]))
        .unwrap();
    assert_eq!(visible(&document, 0), ["Dan"]);
}

#[test]
fn invalid_views_are_rejected() {
    let mut document = open(PEOPLE);
    let bad_regex = ViewSpec {
        sort: vec![],
        filters: vec![filter(Some(0), FilterOp::Matches, "(")],
    };
    assert!(matches!(document.set_view(bad_regex), Err(Error::InvalidPattern(_))));
    assert!(matches!(
        document.set_view(sort(7, true)),
        Err(Error::ColumnOutOfRange { .. })
    ));
}

#[test]
fn edits_through_a_view() {
    let mut document = open(PEOPLE);
    document.set_view(sort(0, true)).unwrap();
    // Visible row 0 is "alice", row 2 of the file.
    let row = document.visible_record(0).unwrap();
    document.set_cell(row, 0, "Zoe").unwrap();
    // An edited row keeps its place until the view is refreshed.
    assert_eq!(visible(&document, 0), ["Zoe", "Bob", "Clara", "Dan"]);

    document.delete_rows(&[document.visible_record(1).unwrap()]).unwrap();
    assert_eq!(visible(&document, 0), ["Clara", "Dan", "Zoe"]);

    document.undo();
    assert_eq!(visible(&document, 0), ["Bob", "Clara", "Dan", "Zoe"]);
}

#[test]
fn apply_sort_reorders_the_file() {
    let mut document = open(b"n\nb\nc\na");
    document.set_view(sort(0, true)).unwrap();
    assert!(document.apply_sort().unwrap());
    assert!(document.view_spec().is_empty());
    assert_eq!(document.to_bytes().unwrap(), b"n\na\nb\nc");
    document.undo();
    assert_eq!(document.to_bytes().unwrap(), b"n\nb\nc\na");
}

#[test]
fn insertion_row_follows_the_view() {
    let mut document = open(PEOPLE);
    assert_eq!(document.insertion_row(0), 1);
    assert_eq!(document.insertion_row(4), 5);
    document
        .set_view(ViewSpec {
            sort: vec![],
            filters: vec![filter(Some(1), FilterOp::Equals, "london")],
        })
        .unwrap();
    assert_eq!(document.insertion_row(1), 3);
    assert_eq!(document.insertion_row(2), 4);
}

#[test]
fn column_changes_reset_the_view() {
    let mut document = open(PEOPLE);
    document.set_view(sort(0, true)).unwrap();
    document.insert_column(0).unwrap();
    assert!(document.view_spec().is_empty());
    assert_eq!(visible(&document, 1), ["Bob", "alice", "Clara", "Dan"]);
}

#[test]
fn find_walks_visible_cells_and_wraps() {
    let mut document = open(PEOPLE);
    let q = query("lon");
    assert_eq!(document.find(&q, None, true).unwrap(), Some((0, 1)));
    assert_eq!(document.find(&q, Some((0, 1)), true).unwrap(), Some((2, 1)));
    assert_eq!(document.find(&q, Some((2, 1)), true).unwrap(), Some((0, 1)));
    assert_eq!(document.find(&q, Some((0, 1)), false).unwrap(), Some((2, 1)));
    assert_eq!(document.find(&q, None, false).unwrap(), Some((2, 1)));
    assert_eq!(document.count_matches(&q).unwrap(), 2);

    let case_sensitive = SearchQuery { case_sensitive: true, ..query("lon") };
    assert_eq!(document.count_matches(&case_sensitive).unwrap(), 1);

    document.set_view(sort(0, false)).unwrap();
    // Dan, Clara, Bob, alice: Clara's london comes first now.
    assert_eq!(document.find(&q, None, true).unwrap(), Some((1, 1)));
}

#[test]
fn find_options() {
    let document = open(PEOPLE);
    let whole = SearchQuery { whole_cell: true, ..query("london") };
    assert_eq!(document.count_matches(&whole).unwrap(), 2);
    let empty_cells = SearchQuery { whole_cell: true, ..query("") };
    assert_eq!(document.count_matches(&empty_cells).unwrap(), 1);
    let regex = SearchQuery { regex: true, ..query(r"^\d+$") };
    assert_eq!(document.count_matches(&regex).unwrap(), 3);
    let in_column = SearchQuery { column: Some(0), ..query("a") };
    assert_eq!(document.count_matches(&in_column).unwrap(), 3);
    assert_eq!(document.count_matches(&query("")).unwrap(), 0);
    assert!(matches!(
        document.count_matches(&SearchQuery { regex: true, ..query("[") }),
        Err(Error::InvalidPattern(_))
    ));
}

#[test]
fn replace_in_one_cell_and_everywhere() {
    let mut document = open(PEOPLE);
    assert!(document.replace_in_cell(0, 1, &query("lon"), "LON").unwrap());
    assert_eq!(document.cell(1, 1).unwrap(), "LONdon");
    assert!(!document.replace_in_cell(1, 1, &query("lon"), "Lon").unwrap());

    let count = document.replace_all(&query("london"), "Paris").unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        document.to_bytes().unwrap(),
        b"name,city,amount\nBob,Paris,10\nalice,Moscow,9\nClara,Paris,100\nDan,,\"1 000,5\"\n"
    );
    // Replace all is one step.
    document.undo();
    assert_eq!(visible(&document, 1), ["LONdon", "Moscow", "london", ""]);
}

#[test]
fn regex_replacement_uses_groups_and_literal_mode_does_not() {
    let mut document = open(b"phone\n8-999-1234567\n");
    let regex = SearchQuery { regex: true, ..query(r"^8-(\d{3})-(\d+)$") };
    document.replace_all(&regex, "+7 ($1) $2").unwrap();
    assert_eq!(document.visible_row(0).unwrap(), ["+7 (999) 1234567"]);

    document.replace_all(&query("+7"), "$1").unwrap();
    assert_eq!(document.visible_row(0).unwrap(), ["$1 (999) 1234567"]);
}

#[test]
fn replace_all_respects_filters() {
    let mut document = open(PEOPLE);
    document
        .set_view(ViewSpec {
            sort: vec![],
            filters: vec![filter(Some(0), FilterOp::Equals, "bob")],
        })
        .unwrap();
    assert_eq!(document.replace_all(&query("london"), "Paris").unwrap(), 1);
    assert_eq!(document.cell(3, 1).unwrap(), "london");
}

#[test]
fn search_inside_a_range() {
    let mut document = Document::from_bytes(b"a,b,c\nx,x,x\nx,x,x\nx,x,x\nx,x,x\n".to_vec());
    // Visible rows 1..=2, columns 1..=2: four cells.
    let range = comma_core::CellRange { top: 1, left: 1, bottom: 2, right: 2 };
    let scoped = SearchQuery { range: Some(range), ..query("x") };
    assert_eq!(document.count_matches(&scoped).unwrap(), 4);
    assert_eq!(document.count_matches(&SearchQuery { column: Some(2), ..scoped.clone() }).unwrap(), 2);
    assert_eq!(document.count_matches(&SearchQuery { column: Some(0), ..scoped.clone() }).unwrap(), 0);

    // Finding wraps around inside the range, and starts there from a cell outside it.
    assert_eq!(document.find(&scoped, None, true).unwrap(), Some((1, 1)));
    assert_eq!(document.find(&scoped, Some((1, 1)), true).unwrap(), Some((1, 2)));
    assert_eq!(document.find(&scoped, Some((2, 2)), true).unwrap(), Some((1, 1)));
    assert_eq!(document.find(&scoped, Some((1, 1)), false).unwrap(), Some((2, 2)));
    assert_eq!(document.find(&scoped, Some((0, 0)), true).unwrap(), Some((1, 1)));

    assert_eq!(document.replace_all(&scoped, "y").unwrap(), 4);
    assert_eq!(
        String::from_utf8(document.to_bytes().unwrap()).unwrap(),
        "a,b,c\nx,x,x\nx,y,y\nx,y,y\nx,x,x\n"
    );

    // A range past the table is cut to it.
    let past = comma_core::CellRange { top: 3, left: 0, bottom: 99, right: 99 };
    assert_eq!(document.count_matches(&SearchQuery { range: Some(past), ..query("x") }).unwrap(), 3);
}
