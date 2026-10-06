use comma_core::{Document, Error, LineEnding, SaveFormat, TextEncoding};
use encoding_rs::WINDOWS_1251;

fn open(bytes: &[u8]) -> Document {
    Document::from_bytes(bytes.to_vec())
}

fn format(encoding: TextEncoding, bom: bool, delimiter: u8, line_ending: LineEnding) -> SaveFormat {
    SaveFormat { encoding, bom, delimiter, line_ending }
}

#[test]
fn the_source_format_stays_byte_exact() {
    let bytes = b"\"a\",b\r\n1,\"2\"\n";
    let document = open(bytes);
    assert_eq!(document.save_format(), document.source_format());
    assert_eq!(document.to_bytes_with(&document.source_format()).unwrap(), bytes);
}

#[test]
fn windows_1251_to_excel_utf8() {
    let (bytes, _, _) = WINDOWS_1251.encode("Имя;Город\nИван;\"Москва; центр\"\n");
    let document = open(&bytes);
    let excel = format(TextEncoding::Utf8, true, b';', LineEnding::CrLf);
    let converted = document.to_bytes_with(&excel).unwrap();
    assert_eq!(converted, "\u{FEFF}Имя;Город\r\nИван;\"Москва; центр\"\r\n".as_bytes());
}

#[test]
fn same_delimiter_keeps_original_quoting() {
    let document = open(b"\"id\",\"name\"\n\"1\",\"Ann\"\n");
    let crlf = format(TextEncoding::Utf8, false, b',', LineEnding::CrLf);
    assert_eq!(document.to_bytes_with(&crlf).unwrap(), b"\"id\",\"name\"\r\n\"1\",\"Ann\"\r\n");
}

#[test]
fn new_delimiter_requotes_values() {
    let mut document = open(b"name;note\nAnn;\"a, b\"\nBob;plain\n");
    document.set_cell(2, 1, "x\"y").unwrap();
    let comma = format(TextEncoding::Utf8, false, b',', LineEnding::Lf);
    assert_eq!(
        document.to_bytes_with(&comma).unwrap(),
        b"name,note\nAnn,\"a, b\"\nBob,\"x\"\"y\"\n"
    );
    let tab = format(TextEncoding::Utf8, false, b'\t', LineEnding::Lf);
    assert_eq!(document.to_bytes_with(&tab).unwrap(), b"name\tnote\nAnn\ta, b\nBob\t\"x\"\"y\"\n");
}

#[test]
fn missing_final_line_break_is_kept() {
    let document = open(b"a\nb");
    let crlf = format(TextEncoding::Utf8, false, b',', LineEnding::CrLf);
    assert_eq!(document.to_bytes_with(&crlf).unwrap(), b"a\r\nb");
}

#[test]
fn utf8_to_utf16_and_back() {
    let document = open("a,б\n1,2\n".as_bytes());
    let utf16 = format(TextEncoding::Utf16Le, true, b',', LineEnding::Lf);
    let bytes = document.to_bytes_with(&utf16).unwrap();
    let mut expected = vec![0xFF, 0xFE];
    for unit in "a,б\n1,2\n".encode_utf16() {
        expected.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(bytes, expected);
    let reopened = open(&bytes);
    assert_eq!(reopened.row(0).unwrap(), ["a", "б"]);
}

#[test]
fn unmappable_values_name_their_cell() {
    let document = open("id,name\n1,Ann 🙂\n".as_bytes());
    let cp1251 = format(TextEncoding::Windows1251, false, b',', LineEnding::Lf);
    assert!(matches!(
        document.to_bytes_with(&cp1251),
        Err(Error::Unmappable { row: 1, column: 1, .. })
    ));
}

#[test]
fn chosen_format_is_used_for_later_saves() {
    let mut document = open(b"a,b\n1,2\n");
    let semicolons = format(TextEncoding::Utf8, false, b';', LineEnding::Lf);
    document.set_save_format(semicolons).unwrap();
    assert_eq!(document.info().dialect.delimiter, b';');
    assert_eq!(document.to_bytes().unwrap(), b"a;b\n1;2\n");
    document.set_cell(1, 0, "x").unwrap();
    assert_eq!(document.to_bytes().unwrap(), b"a;b\nx;2\n");
}

#[test]
fn invalid_delimiters_are_rejected() {
    let mut document = open(b"a\n");
    for delimiter in [b'"', b'\n', 0xD0] {
        let bad = format(TextEncoding::Utf8, false, delimiter, LineEnding::Lf);
        assert!(matches!(document.set_save_format(bad), Err(Error::InvalidDelimiter(_))));
    }
}

#[test]
fn bom_is_ignored_for_legacy_encodings() {
    let document = open(b"a\n");
    let cp1251 = format(TextEncoding::Windows1251, true, b',', LineEnding::Lf);
    assert_eq!(document.to_bytes_with(&cp1251).unwrap(), b"a\n");
}
