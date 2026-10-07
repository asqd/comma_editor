use comma_core::{Document, Fix, ProblemKind};

fn open(bytes: &str) -> Document {
    Document::from_bytes(bytes.as_bytes().to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

fn kinds(document: &Document) -> Vec<(usize, ProblemKind)> {
    document
        .find_problems(100)
        .problems
        .iter()
        .map(|problem| (problem.row, problem.kind))
        .collect()
}

fn fixes(document: &Document, row: usize) -> Vec<Fix> {
    document
        .repair_options(row)
        .unwrap()
        .into_iter()
        .map(|option| option.fix)
        .collect()
}

#[test]
fn clean_files_have_no_problems() {
    let document = open("id,note\n1,\"Line one\nline two\"\n2,\"a, b\"\n");
    let report = document.find_problems(100);
    assert_eq!(report.expected_fields, 2);
    assert!(report.problems.is_empty());
    assert_eq!(report.total(), 0);
    assert!(document.repair_options(1).unwrap().is_empty());
}

#[test]
fn pads_a_short_row() {
    let original = "a,b,c\n1,2,3\n4,5\n6,7,8\n";
    let mut document = open(original);
    let report = document.find_problems(100);
    assert_eq!(report.expected_fields, 3);
    assert_eq!(report.problems.len(), 1);
    assert_eq!(report.problems[0].fields, 2);
    assert_eq!(kinds(&document), vec![(2, ProblemKind::MissingFields)]);
    assert_eq!(fixes(&document, 2), vec![Fix::PadFields, Fix::DeleteRow]);

    let options = document.repair_options(2).unwrap();
    assert_eq!(options[0].rows, vec![vec!["4", "5", ""]]);
    assert_eq!(options[1].row_count, 0);

    assert!(document.apply_fix(2, Fix::PadFields).unwrap());
    assert_eq!(saved(&document), "a,b,c\n1,2,3\n4,5,\n6,7,8\n");
    assert!(kinds(&document).is_empty());
    document.undo();
    assert_eq!(saved(&document), original);
    assert!(!document.apply_fix(1, Fix::PadFields).unwrap());
}

#[test]
fn joins_a_row_split_by_a_line_break() {
    let original = "id,note,city\n1,first line\nsecond line,Moscow\n2,ok,Paris\n";
    let mut document = open(original);
    assert_eq!(
        kinds(&document),
        vec![(1, ProblemKind::MissingFields), (2, ProblemKind::MissingFields)]
    );
    assert_eq!(fixes(&document, 1), vec![Fix::JoinNext, Fix::PadFields, Fix::DeleteRow]);
    assert_eq!(document.fix_all(ProblemKind::MissingFields), 1);
    assert_eq!(saved(&document), "id,note,city\n1,\"first line\nsecond line\",Moscow\n2,ok,Paris\n");
    assert_eq!(document.row_count(), 3);
    document.undo();
    assert_eq!(saved(&document), original);
}

#[test]
fn deletes_blank_lines() {
    let mut document = open("a,b\n1,2\n\n3,4\n\n");
    assert_eq!(
        kinds(&document),
        vec![(2, ProblemKind::MissingFields), (4, ProblemKind::MissingFields)]
    );
    assert_eq!(fixes(&document, 2), vec![Fix::DeleteRow, Fix::PadFields]);
    assert_eq!(document.fix_all(ProblemKind::MissingFields), 2);
    assert_eq!(saved(&document), "a,b\n1,2\n3,4\n");
}

#[test]
fn merges_extra_fields_where_they_fit() {
    let original = "id,name,age\n1,Ann,30\n2,Smith, John,40\n3,Bob,50\n";
    let mut document = open(original);
    assert_eq!(document.column_count(), 4);
    assert_eq!(kinds(&document), vec![(2, ProblemKind::ExtraFields)]);
    assert_eq!(fixes(&document, 2)[0], Fix::MergeFields { column: 1 });
    assert_eq!(*fixes(&document, 2).last().unwrap(), Fix::DeleteRow);

    assert!(document.apply_fix(2, Fix::MergeFields { column: 1 }).unwrap());
    assert_eq!(saved(&document), "id,name,age\n1,Ann,30\n2,\"Smith, John\",40\n3,Bob,50\n");
    assert_eq!(document.column_count(), 3);
    document.undo();
    assert_eq!(document.column_count(), 4);
    assert_eq!(saved(&document), original);
}

#[test]
fn merges_a_number_split_at_its_decimal_comma() {
    let mut document = open("id,price,city\n1,10.5,Moscow\n2,12,5,Paris\n3,7.25,Kazan\n");
    assert_eq!(fixes(&document, 2)[0], Fix::MergeFields { column: 1 });
    document.fix_all(ProblemKind::ExtraFields);
    assert_eq!(document.row(2).unwrap(), vec!["2", "12,5", "Paris"]);
}

#[test]
fn drops_empty_trailing_fields() {
    let mut document = open("a,b\n1,2,,\n3,4\n");
    assert_eq!(fixes(&document, 1)[0], Fix::DropEmptyFields);
    document.fix_all(ProblemKind::ExtraFields);
    assert_eq!(saved(&document), "a,b\n1,2\n3,4\n");
    assert_eq!(document.column_count(), 2);
}

#[test]
fn keeps_an_unclosed_quote_as_text() {
    let original = "a,b\n1,\"open\n2,x\n3,y\n";
    let mut document = open(original);
    assert_eq!(document.row_count(), 2);
    let report = document.find_problems(100);
    assert_eq!(report.problems[0].kind, ProblemKind::UnclosedQuote);
    assert_eq!(report.problems[0].lines, 3);
    assert_eq!(fixes(&document, 1), vec![Fix::QuoteAsText, Fix::RemoveQuote]);

    let options = document.repair_options(1).unwrap();
    assert_eq!(options[0].row_count, 3);
    assert_eq!(options[0].rows[0], vec!["1", "\"open"]);
    assert_eq!(options[1].rows[0], vec!["1", "open"]);

    assert!(document.apply_fix(1, Fix::QuoteAsText).unwrap());
    assert_eq!(saved(&document), "a,b\n1,\"\"\"open\"\n2,x\n3,y\n");
    assert_eq!(document.row_count(), 4);
    assert!(kinds(&document).is_empty());
    document.undo();
    assert_eq!(saved(&document), original);
    document.redo();
    assert_eq!(saved(&document), "a,b\n1,\"\"\"open\"\n2,x\n3,y\n");
    document.undo();

    assert!(document.apply_fix(1, Fix::RemoveQuote).unwrap());
    assert_eq!(saved(&document), "a,b\n1,open\n2,x\n3,y\n");
}

#[test]
fn unclosed_quote_without_a_final_line_break() {
    let mut document = open("a,b\r\n1,\"x\r\n2,y");
    document.fix_all(ProblemKind::UnclosedQuote);
    assert_eq!(saved(&document), "a,b\r\n1,\"\"\"x\"\r\n2,y");
}

#[test]
fn splits_rows_swallowed_by_a_stray_quote() {
    let original = "a,b,c\n1,\"abc,2\n3,x\"y,4\n5,6,7\n";
    let mut document = open(original);
    assert_eq!(document.row_count(), 3);
    assert_eq!(kinds(&document), vec![(1, ProblemKind::StrayQuote)]);
    document.fix_all(ProblemKind::StrayQuote);
    assert_eq!(saved(&document), "a,b,c\n1,\"\"\"abc\",2\n3,x\"y,4\n5,6,7\n");
    assert_eq!(document.row(2).unwrap(), vec!["3", "x\"y", "4"]);
    assert!(kinds(&document).is_empty());
}

#[test]
fn reports_counts_beyond_the_limit() {
    let mut text = String::from("a,b\n");
    for _ in 0..30 {
        text.push_str("1,2\n1\n");
    }
    let document = open(&text);
    let report = document.find_problems(10);
    assert_eq!(report.problems.len(), 10);
    assert_eq!(report.counts, vec![(ProblemKind::MissingFields, 30)]);
}

#[test]
fn splits_fields_merged_by_a_stray_quote_in_one_row() {
    let mut document = open("a,b,c,d\n\"1,2,\"x\",4\n5,6,7,8\n");
    assert_eq!(document.row(1).unwrap(), vec!["1,2,x\"", "4"]);
    assert_eq!(kinds(&document), vec![(1, ProblemKind::StrayQuote)]);
    document.fix_all(ProblemKind::StrayQuote);
    assert_eq!(document.row(1).unwrap(), vec!["\"1", "2", "x", "4"]);
    assert_eq!(saved(&document), "a,b,c,d\n\"\"\"1\",2,\"x\",4\n5,6,7,8\n");
}

#[test]
fn shows_rows_as_written() {
    let document = open("a,b\n1,\"x\ny\"\n");
    assert_eq!(document.raw_text(1, 100).unwrap(), "1,\"x\ny\"");
    assert_eq!(document.raw_text(1, 3).unwrap(), "1,\"…");
}

#[test]
fn merges_where_values_are_typical_for_their_columns() {
    let mut document = open("id,name,city,amount\n1,Ann,Moscow,10.5\n4,Kim,Seoul,12,5\n5,Eve,Rome,40\n7,Max,Berlin,60\n");
    assert_eq!(fixes(&document, 2)[0], Fix::MergeFields { column: 3 });
    document.fix_all(ProblemKind::ExtraFields);
    assert_eq!(document.row(2).unwrap(), vec!["4", "Kim", "Seoul", "12,5"]);
}
