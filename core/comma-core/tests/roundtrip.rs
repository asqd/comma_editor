use comma_core::{Document, Error, LineEnding, QuoteStyle, TextEncoding};
use encoding_rs::{KOI8_R, WINDOWS_1251};

fn open(bytes: &[u8]) -> Document {
    Document::from_bytes(bytes.to_vec())
}

fn saved(document: &Document) -> Vec<u8> {
    document.to_bytes().expect("document saves")
}

fn assert_roundtrip(bytes: &[u8]) {
    let document = open(bytes);
    assert_eq!(
        saved(&document),
        bytes,
        "round trip changed {:?}",
        String::from_utf8_lossy(bytes)
    );
}

#[test]
fn unedited_files_are_byte_identical() {
    let cases: &[&[u8]] = &[
        b"",
        b"\n",
        b"\n\n",
        b"a",
        b"a,b\n1,2\n",
        b"a;b\r\n1;2",
        b"a,b\r1,2\r",
        b"mixed\nline\r\nendings\rhere",
        b"\xEF\xBB\xBFid,name\n1,Alice\n",
        b"\"quoted\",\"with \"\"quotes\"\"\"\n\"multi\nline\",x\n",
        b"ragged,row\nshort\nmuch,longer,row,here\n",
        b"blank\n\nlines\n\n",
        b"a,\"unterminated\nquote,b\n",
        b"trailing,delimiter,\n,leading\n",
        b"\"closed at eof\"",
        b"tab\tseparated\nvalues\there\n",
        b"  spaced , values  \n",
    ];
    for case in cases {
        assert_roundtrip(case);
    }
}

#[test]
fn legacy_and_utf16_encodings_round_trip() {
    let (cp1251, _, _) = WINDOWS_1251.encode("Имя;Город\nИван;Москва\nОльга;Санкт-Петербург\n");
    let document = open(&cp1251);
    assert_eq!(document.info().encoding, TextEncoding::Windows1251);
    assert_eq!(document.row(1).unwrap(), vec!["Иван", "Москва"]);
    assert_eq!(saved(&document), cp1251.as_ref());

    let (koi8, _, _) = KOI8_R.encode("имя,город\nиван,москва\nольга,казань\n");
    let document = open(&koi8);
    assert_eq!(document.info().encoding, TextEncoding::Koi8R);
    assert_eq!(saved(&document), koi8.as_ref());

    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "id,город\r\n1,Тверь\r\n".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let document = open(&utf16);
    let info = document.info();
    assert_eq!(info.encoding, TextEncoding::Utf16Le);
    assert!(info.has_bom);
    assert_eq!(info.dialect.line_ending, LineEnding::CrLf);
    assert_eq!(document.row(1).unwrap(), vec!["1", "Тверь"]);
    assert_eq!(saved(&document), utf16);
}

/// Pseudo-random files built from bytes that matter to CSV must all survive a round trip.
#[test]
fn random_inputs_round_trip() {
    const ALPHABET: &[u8] = &[
        b',', b';', b'\t', b'|', b'"', b'"', b'\n', b'\r', b' ', b'a', b'b', b'1', 0xD0, 0xB0,
        0xC3, 0xA9, 0xE9, 0xFF, 0xFE, 0x00, 0xEF, 0xBB, 0xBF,
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let len = (next() % 64) as usize;
        let bytes: Vec<u8> = (0..len)
            .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
            .collect();
        assert_roundtrip(&bytes);
    }
}

#[test]
fn editing_a_cell_changes_only_that_field() {
    let mut document = open(b"\"id\",\"name\",\"note\"\n1,\"Alice\",\"keeps, quotes\"\n2,Bob,x\n");
    assert!(document.set_cell(1, 1, "Alicia").unwrap());
    assert_eq!(
        saved(&document),
        b"\"id\",\"name\",\"note\"\n1,Alicia,\"keeps, quotes\"\n2,Bob,x\n"
    );
}

#[test]
fn setting_the_same_value_records_nothing() {
    let mut document = open(b"a,b\n1,2\n");
    assert!(!document.set_cell(1, 0, "1").unwrap());
    assert!(!document.can_undo());
}

#[test]
fn new_values_are_quoted_when_needed() {
    let mut document = open(b"a;b\n1;2\n");
    document.set_cell(1, 0, "x;y").unwrap();
    document.set_cell(1, 1, "say \"hi\"\nbye").unwrap();
    assert_eq!(saved(&document), b"a;b\n\"x;y\";\"say \"\"hi\"\"\nbye\"\n");
}

#[test]
fn quote_all_files_keep_quoting_new_values() {
    let mut document = open(b"\"a\",\"b\"\n\"1\",\"2\"\n");
    assert_eq!(document.info().dialect.quote_style, QuoteStyle::Always);
    document.set_cell(1, 1, "3").unwrap();
    assert_eq!(saved(&document), b"\"a\",\"b\"\n\"1\",\"3\"\n");
}

#[test]
fn editing_beyond_a_short_row_pads_it() {
    let mut document = open(b"a,b,c\nx\n");
    document.set_cell(1, 2, "z").unwrap();
    assert_eq!(saved(&document), b"a,b,c\nx,,z\n");
}

#[test]
fn appending_rows_respects_missing_trailing_newline() {
    let mut document = open(b"a,b\r\n1,2");
    document.insert_rows(2, 1).unwrap();
    document.set_cell(2, 0, "3").unwrap();
    assert_eq!(saved(&document), b"a,b\r\n1,2\r\n3,");
    document.undo();
    document.undo();
    assert_eq!(saved(&document), b"a,b\r\n1,2");
}

#[test]
fn deleting_rows_is_one_undoable_step() {
    let original = b"r0\nr1\nr2\nr3\nr4\nr5\n";
    let mut document = open(original);
    document.delete_rows(&[5, 1, 2, 4]).unwrap();
    assert_eq!(saved(&document), b"r0\nr3\n");
    assert!(document.undo());
    assert_eq!(saved(&document), original);
    assert!(document.redo());
    assert_eq!(saved(&document), b"r0\nr3\n");
}

#[test]
fn deleting_every_row_keeps_the_bom() {
    let mut document = open(b"\xEF\xBB\xBFa\nb\n");
    document.delete_rows(&[0, 1]).unwrap();
    assert_eq!(saved(&document), b"\xEF\xBB\xBF");
}

#[test]
fn inserting_and_deleting_columns() {
    let original = b"a,b\n1,2\n\nshort\n";
    let mut document = open(original);
    document.insert_column(1).unwrap();
    assert_eq!(document.column_count(), 3);
    assert_eq!(saved(&document), b"a,,b\n1,,2\n\nshort\n");

    document.delete_column(0).unwrap();
    assert_eq!(saved(&document), b",b\n,2\n\n\n");
    assert_eq!(document.column_count(), 2);

    document.undo();
    document.undo();
    assert_eq!(saved(&document), original);
    assert_eq!(document.column_count(), 2);
}

#[test]
fn appending_a_column_extends_full_rows() {
    let mut document = open(b"a,b\n1,2\nshort\n");
    document.insert_column(2).unwrap();
    assert_eq!(saved(&document), b"a,b,\n1,2,\nshort\n");
}

#[test]
fn the_last_column_cannot_be_deleted() {
    let mut document = open(b"only\nvalues\n");
    assert!(matches!(document.delete_column(0), Err(Error::LastColumn)));
}

#[test]
fn new_edits_clear_redo() {
    let mut document = open(b"a\n");
    document.set_cell(0, 0, "b").unwrap();
    document.undo();
    assert!(document.can_redo());
    document.set_cell(0, 0, "c").unwrap();
    assert!(!document.can_redo());
}

#[test]
fn legacy_encodings_reject_unmappable_values() {
    let (cp1251, _, _) = WINDOWS_1251.encode("имя\nзначение\n");
    let mut document = open(&cp1251);
    document.set_cell(1, 0, "новое").unwrap();
    let (expected, _, _) = WINDOWS_1251.encode("имя\nновое\n");
    assert_eq!(saved(&document), expected.as_ref());

    document.set_cell(1, 0, "emoji 🙂").unwrap();
    assert!(matches!(
        document.to_bytes(),
        Err(Error::Unmappable {
            row: 1,
            column: 0,
            ..
        })
    ));
}

#[test]
fn utf16_edits_are_saved_as_utf16() {
    let encode = |text: &str| {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes
    };
    let mut document = open(&encode("a,b\n1,2\n"));
    document.set_cell(1, 1, "два").unwrap();
    assert_eq!(saved(&document), encode("a,b\n1,два\n"));
}

#[test]
fn header_detection() {
    assert!(open(b"id,name\n1,Alice\n").has_header());
    assert!(!open(b"1,Alice\n2,Bob\n").has_header());
    assert!(!open(b"id,name\n").has_header());
}

#[test]
fn new_document_saves_as_plain_csv() {
    let mut document = Document::new_empty();
    document.set_cell(0, 0, "a").unwrap();
    document.set_cell(0, 2, "c").unwrap();
    assert_eq!(saved(&document), b"a,,c\n");
}
