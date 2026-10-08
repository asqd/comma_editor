use comma_core::{CellRange, Document, Edit, Progress, SearchQuery, TextTransform};

fn open(bytes: &str) -> Document {
    Document::from_bytes(bytes.as_bytes().to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

/// A file large enough to span several blocks of background work.
fn many_rows() -> String {
    let mut text = String::from("id,name,city\n");
    for n in 0..40_000 {
        text.push_str(&format!("{n},name {},{}\n", n % 7, ["Oslo", "Rome", "Lima"][n % 3]));
    }
    text
}

fn edits() -> Vec<Edit> {
    let all = CellRange { top: 0, left: 0, bottom: 39_999, right: 2 };
    vec![
        Edit::ReplaceAll {
            query: SearchQuery { text: "o".into(), ..SearchQuery::default() },
            replacement: "0".into(),
        },
        Edit::Transform { range: all, transform: TextTransform::Uppercase },
        Edit::FillEmpty { range: all, from_above: true, value: String::new() },
        Edit::FixGarbled { range: None },
        Edit::AddFormulaColumn { formula: "{id} * 2".into(), name: "double".into(), at: 1 },
        Edit::FillWithFormula { formula: "UPPER({city})".into(), column: 2 },
        Edit::SplitColumn { column: 1, separator: " ".into(), regex: false },
        Edit::MergeColumns { columns: vec![1, 2], separator: "/".into() },
        Edit::RemoveDuplicates { columns: vec![1, 2], normalize: false },
    ]
}

#[test]
fn background_edits_match_edits_made_at_once() {
    let original = many_rows();
    for edit in edits() {
        let mut at_once = open(&original);
        let expected = at_once.apply_edit(&edit).unwrap();

        let mut document = open(&original);
        let mut task = document.begin_edit(edit.clone());
        let progress = Progress::default();
        assert!(task.run(&progress), "{edit:?}");
        assert_eq!(progress.fraction(), 1.0, "{edit:?}");
        let outcome = document.finish_edit(task).unwrap();
        assert_eq!(outcome.as_ref(), Some(&expected), "{edit:?}");
        assert_eq!(saved(&document), saved(&at_once), "{edit:?}");

        // One undoable step either way.
        if expected.count > 0 {
            assert!(document.undo());
            assert_eq!(saved(&document), original, "{edit:?}");
            assert!(!document.can_undo());
        }
    }
}

#[test]
fn cancelled_edit_changes_nothing() {
    let original = many_rows();
    let mut document = open(&original);
    let mut task = document.begin_edit(edits().remove(1));
    let progress = Progress::default();
    progress.cancel();
    assert!(!task.run(&progress));
    assert_eq!(document.finish_edit(task).unwrap(), None);
    assert_eq!(saved(&document), original);
    assert!(!document.can_undo());
}

#[test]
fn edit_of_rows_that_changed_meanwhile_is_dropped() {
    let mut document = open("a,b\n1,x\n2,y\n");
    let mut task = document.begin_edit(Edit::Transform {
        range: CellRange { top: 0, left: 0, bottom: 1, right: 1 },
        transform: TextTransform::Uppercase,
    });
    document.set_cell(1, 1, "z").unwrap();
    assert!(task.run(&Progress::default()));
    assert_eq!(document.finish_edit(task).unwrap(), None);
    assert_eq!(saved(&document), "a,b\n1,z\n2,y\n");
}

#[test]
fn errors_come_back_when_the_edit_finishes() {
    let mut document = open("a,b\n1,2\n");
    let mut task = document.begin_edit(Edit::AddFormulaColumn {
        formula: "{nope} + 1".into(),
        name: "c".into(),
        at: 2,
    });
    assert!(task.run(&Progress::default()));
    assert!(document.finish_edit(task).is_err());
    assert!(!document.can_undo());
}

#[test]
fn scattered_duplicates_are_removed_and_restored_exactly() {
    let mut text = String::from("k,v\n");
    for n in 0..5_000 {
        // Every third row repeats a key seen before; some rows are modified first.
        text.push_str(&format!("{},\"v {n}\"\r\n", if n % 3 == 2 { n - 2 } else { n }));
    }
    let mut document = open(&text);
    document.set_cell(10, 1, "changed").unwrap();
    let before = saved(&document);
    let removed = document.remove_duplicates(&[0], false).unwrap();
    assert_eq!(removed, 5_000 / 3);
    assert_eq!(document.row_count(), 1 + 5_000 - removed);
    let after = saved(&document);
    assert!(document.undo());
    assert_eq!(saved(&document), before);
    assert!(document.redo());
    assert_eq!(saved(&document), after);
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(saved(&document), text);
}

#[test]
fn undo_history_keeps_to_its_memory_limit() {
    let original = many_rows();
    let all = CellRange { top: 0, left: 0, bottom: 39_999, right: 2 };
    let steps = [TextTransform::Uppercase, TextTransform::Lowercase, TextTransform::Capitalize];

    let mut document = open(&original);
    for transform in steps {
        document.transform_range(all, transform).unwrap();
    }
    for _ in steps {
        assert!(document.undo());
    }
    assert_eq!(saved(&document), original);

    // Each step holds the 40 000 rows it replaced; a small limit keeps only the last one.
    let mut document = open(&original);
    document.set_undo_memory_limit(1 << 20);
    for transform in steps {
        document.transform_range(all, transform).unwrap();
    }
    let last = saved(&document);
    assert!(document.undo());
    assert!(!document.can_undo());
    assert!(document.redo());
    assert_eq!(saved(&document), last);
}
