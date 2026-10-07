use comma_core::{Document, OpenOptions, TextEncoding};

fn cp1251(text: &str) -> Vec<u8> {
    encoding_rs::WINDOWS_1251.encode(text).0.into_owned()
}

#[test]
fn clear_files_are_detected_with_confidence() {
    assert!(Document::from_bytes(b"a,b\n1,2\n".to_vec()).info().detection_confident);
    assert!(Document::from_bytes("имя;город\nИван;Москва\n".as_bytes().to_vec()).info().detection_confident);
    let russian = cp1251("Имя;Город\nИван;Москва\nОльга;Новосибирск\nПётр;Сочи\n");
    let info = Document::from_bytes(russian).info();
    assert_eq!(info.encoding, TextEncoding::Windows1251);
    assert!(info.detection_confident);
}

#[test]
fn unclear_files_are_not() {
    // A few bytes that are not UTF-8 and say little about the language.
    let info = Document::from_bytes(vec![b'a', b',', 0xE9, 0xF1, 0xFC, 0xE4, b'\n', b'1', b',', 0xD6, b'\n']).info();
    assert!(!info.detection_confident);
    // Fields split differently by every candidate delimiter.
    assert!(!Document::from_bytes(b"a,b;c\nd;e\nf,g,h\ni\n".to_vec()).info().detection_confident);
}

#[test]
fn opens_with_a_chosen_encoding_and_delimiter() {
    let bytes = cp1251("Имя;Город\nИван;Москва\n");
    let options = OpenOptions { encoding: Some(TextEncoding::Koi8R), delimiter: Some(b',') };
    let document = Document::from_bytes_with(bytes.clone(), &options);
    let info = document.info();
    assert_eq!(info.encoding, TextEncoding::Koi8R);
    assert_eq!(info.dialect.delimiter, b',');
    assert!(info.detection_confident);
    assert_eq!(document.column_count(), 1);
    // Unchanged, it saves byte for byte in the chosen encoding.
    assert_eq!(document.to_bytes().unwrap(), bytes);

    let document = Document::from_bytes_with(bytes, &OpenOptions { encoding: Some(TextEncoding::Windows1251), delimiter: None });
    assert_eq!(document.row(1).unwrap(), vec!["Иван", "Москва"]);
}

#[test]
fn utf16_that_does_not_decode_falls_back_to_detection() {
    // An odd number of bytes cannot be UTF-16.
    let document = Document::from_bytes_with(b"a,b\n1,22\n".to_vec(), &OpenOptions { encoding: Some(TextEncoding::Utf16Le), delimiter: None });
    assert_eq!(document.info().encoding, TextEncoding::Utf8);
    assert_eq!(document.row(1).unwrap(), vec!["1", "22"]);
}

#[test]
fn previews_the_start_of_a_file() {
    let dir = std::env::temp_dir().join(format!("comma-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("people.csv");
    let mut text = String::from("name;city\n");
    for i in 0..1000 {
        text.push_str(&format!("Имя {i};Город {i}\n"));
    }
    std::fs::write(&path, cp1251(&text)).unwrap();

    let preview = Document::preview_file(&path, &OpenOptions::default(), 5, 4096).unwrap();
    assert_eq!(preview.encoding, TextEncoding::Windows1251);
    assert_eq!(preview.delimiter, b';');
    assert!(preview.has_header);
    assert_eq!(preview.rows.len(), 5);
    assert_eq!(preview.rows[1], vec!["Имя 0", "Город 0"]);

    let preview = Document::preview_file(&path, &OpenOptions { encoding: Some(TextEncoding::Utf8), delimiter: Some(b',') }, 3, 4096).unwrap();
    assert_eq!(preview.column_count, 1);
    assert!(preview.rows[1][0].contains('\u{FFFD}'));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fixes_garbled_text_in_the_whole_file() {
    let text = "имя,город\nИван,Москва\nPlain,ascii\n";
    let garbled = encoding_rs::WINDOWS_1251.decode(text.as_bytes()).0.into_owned();
    let mut document = Document::from_bytes(garbled.clone().into_bytes());
    assert!(document.find_problems(10).garbled_sample >= 4);
    assert_eq!(document.fix_garbled_text(None).unwrap(), 4);
    assert_eq!(String::from_utf8(document.to_bytes().unwrap()).unwrap(), text);
    document.undo();
    assert_eq!(String::from_utf8(document.to_bytes().unwrap()).unwrap(), garbled);
}
