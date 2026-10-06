use comma_core::{CellRange, Document, TextTransform};

fn open(bytes: &[u8]) -> Document {
    Document::from_bytes(bytes.to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

fn range(top: usize, left: usize, bottom: usize, right: usize) -> CellRange {
    CellRange { top, left, bottom, right }
}

#[test]
fn transform_a_range() {
    let mut document = open(b"name,city\n  ann smith ,MOSCOW\nBOB,  paris\n");
    assert_eq!(document.transform_range(range(0, 0, 1, 1), TextTransform::Trim).unwrap(), 2);
    document.transform_range(range(0, 0, 1, 0), TextTransform::Capitalize).unwrap();
    document.transform_range(range(0, 1, 1, 1), TextTransform::Lowercase).unwrap();
    assert_eq!(saved(&document), "name,city\nAnn Smith,moscow\nBob,paris\n");
    document.undo();
    document.undo();
    document.undo();
    assert_eq!(saved(&document), "name,city\n  ann smith ,MOSCOW\nBOB,  paris\n");
}

#[test]
fn fill_empty_cells_from_above_or_with_a_value() {
    let original = "group,item\nA,1\n,2\n ,3\nB,4\n,5\n";
    let mut document = open(original.as_bytes());
    assert_eq!(document.fill_empty(range(1, 0, 4, 0), true, "").unwrap(), 3);
    assert_eq!(saved(&document), "group,item\nA,1\nA,2\nA,3\nB,4\nB,5\n");
    document.undo();
    assert_eq!(document.fill_empty(range(0, 0, 4, 0), false, "—").unwrap(), 3);
    assert_eq!(saved(&document), "group,item\nA,1\n—,2\n—,3\nB,4\n—,5\n");
}

#[test]
fn split_a_column() {
    let mut document = open("name,age\nIvan Petrov,30\nAnna,25\nOlga Ivanovna Smirnova,40\n".as_bytes());
    assert_eq!(document.split_column(0, " ", false).unwrap(), 3);
    assert_eq!(
        saved(&document),
        "name,name 1,name 2,name 3,age\n\
         Ivan Petrov,Ivan,Petrov,,30\n\
         Anna,Anna,,,25\n\
         Olga Ivanovna Smirnova,Olga,Ivanovna,Smirnova,40\n"
    );
    assert_eq!(document.column_count(), 5);
    document.undo();
    assert_eq!(document.column_count(), 2);

    assert_eq!(document.split_column(1, " ", false).unwrap(), 0);
    let mut emails = open(b"user@example.com;x\nme@host.org;y\n");
    emails.set_has_header(false);
    assert_eq!(emails.split_column(0, "[@.]", true).unwrap(), 3);
    assert_eq!(saved(&emails), "user@example.com;user;example;com;x\nme@host.org;me;host;org;y\n");
}

#[test]
fn merge_columns() {
    let mut document = open(b"first,last,age\nIvan,Petrov,30\nAnna,,25\n");
    document.merge_columns(&[1, 0], " ").unwrap();
    assert_eq!(
        saved(&document),
        "first,last,first last,age\nIvan,Petrov,Ivan Petrov,30\nAnna,,Anna,25\n"
    );
}

#[test]
fn remove_duplicate_rows() {
    let original = "email,name\na@x.com,Ann\nb@x.com,Bob\nA@X.com ,Ann\na@x.com,Ann\nb@x.com,Robert\n";
    let mut document = open(original.as_bytes());
    assert_eq!(document.remove_duplicates(&[], false).unwrap(), 1);
    assert_eq!(saved(&document), "email,name\na@x.com,Ann\nb@x.com,Bob\nA@X.com ,Ann\nb@x.com,Robert\n");
    document.undo();
    assert_eq!(document.remove_duplicates(&[0], true).unwrap(), 3);
    assert_eq!(saved(&document), "email,name\na@x.com,Ann\nb@x.com,Bob\n");
    document.undo();
    assert_eq!(saved(&document), original);
}
