use comma_core::Document;

fn open(bytes: &str) -> Document {
    Document::from_bytes(bytes.as_bytes().to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

#[test]
fn moves_rows_down_and_up() {
    let original = "h\na\nb\nc\nd\n";
    let mut document = open(original);
    assert!(document.move_rows(&[1, 2], 4).unwrap());
    assert_eq!(saved(&document), "h\nc\na\nb\nd\n");
    document.undo();
    assert_eq!(saved(&document), original);

    assert!(document.move_rows(&[4], 1).unwrap());
    assert_eq!(saved(&document), "h\nd\na\nb\nc\n");
    document.undo();

    assert!(document.move_rows(&[1, 3], 5).unwrap());
    assert_eq!(saved(&document), "h\nb\nd\na\nc\n");
    document.undo();
    document.redo();
    assert_eq!(saved(&document), "h\nb\nd\na\nc\n");
}

#[test]
fn moving_rows_onto_themselves_does_nothing() {
    let mut document = open("h\na\nb\nc\n");
    assert!(!document.move_rows(&[1, 2], 1).unwrap());
    assert!(!document.move_rows(&[1, 2], 3).unwrap());
    assert!(!document.can_undo());
    assert!(document.move_rows(&[5], 1).is_err());
}

#[test]
fn moves_the_last_row_without_a_line_break() {
    let mut document = open("h\na\nb");
    document.move_rows(&[2], 1).unwrap();
    assert_eq!(saved(&document), "h\nb\na");
}

#[test]
fn moves_a_column() {
    let original = "a,b,c\n1,2,3\n4\n\n7,8,9\n";
    let mut document = open(original);
    assert!(document.move_column(0, 2).unwrap());
    // "4" belongs to column a, which is now the last one.
    assert_eq!(saved(&document), "b,c,a\n2,3,1\n,,4\n\n8,9,7\n");
    document.undo();
    assert_eq!(saved(&document), original);
    assert!(document.move_column(2, 0).unwrap());
    assert_eq!(saved(&document), "c,a,b\n3,1,2\n,4,\n\n9,7,8\n");
    assert!(!document.move_column(1, 1).unwrap());
}
