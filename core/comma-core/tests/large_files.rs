use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use comma_core::{Document, OpenOptions, SortKey, ViewSpec};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("comma-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn people(rows: usize, final_newline: bool) -> String {
    let mut text = String::from("id,name,note\n");
    for i in 0..rows {
        text.push_str(&format!("{i},Name {},\"multi\nline {i}\"\n", rows - i));
    }
    if !final_newline {
        text.pop();
    }
    text
}

#[test]
fn indexes_in_steps_like_a_full_open() {
    let dir = temp_dir("partial");
    for final_newline in [true, false] {
        let text = people(5000, final_newline);
        let path = dir.join("people.csv");
        std::fs::write(&path, &text).unwrap();

        let full = Document::open(&path).unwrap();
        let (mut document, indexer) = Document::open_partially(&path, &OpenOptions::default(), 1000).unwrap();
        let mut indexer = indexer.expect("a file larger than the first part");
        assert!(document.is_indexing());
        assert!(document.row_count() < full.row_count());
        assert!(document.has_header());
        loop {
            let chunk = indexer.next_chunk(7777);
            let done = chunk.is_done();
            document.append_index(chunk);
            if done {
                break;
            }
        }
        assert_eq!(indexer.progress(), (text.len(), text.len()));
        assert!(!document.is_indexing());
        assert_eq!(document.row_count(), full.row_count());
        assert_eq!(document.row(4000).unwrap(), full.row(4000).unwrap());
        assert_eq!(document.info().trailing_newline, final_newline);
        assert_eq!(document.to_bytes().unwrap(), text.as_bytes());
    }
    // A small file is indexed at once.
    let path = dir.join("small.csv");
    std::fs::write(&path, "a,b\n1,2\n").unwrap();
    let (document, indexer) = Document::open_partially(&path, &OpenOptions::default(), 1 << 20).unwrap();
    assert!(indexer.is_none());
    assert!(!document.is_indexing());
    std::fs::remove_dir_all(dir).unwrap();
}

fn sorted_by_name() -> ViewSpec {
    ViewSpec { sort: vec![SortKey { column: 1, ascending: true }], filters: Vec::new() }
}

#[test]
fn sorts_off_the_document() {
    let mut expected = Document::from_bytes(people(300, true).into_bytes());
    expected.set_view(sorted_by_name()).unwrap();

    let mut document = Document::from_bytes(people(300, true).into_bytes());
    let mut task = document.begin_view(sorted_by_name()).unwrap();
    assert!(task.run(&AtomicBool::new(false)));
    assert!(document.finish_view(task));
    let names = |document: &Document| (0..5).map(|row| document.visible_row(row).unwrap()[1].clone()).collect::<Vec<_>>();
    assert_eq!(names(&document), names(&expected));
    assert_eq!(names(&document)[0], "Name 1");
}

#[test]
fn a_view_task_is_dropped_when_cancelled_or_out_of_date() {
    let mut document = Document::from_bytes(people(300, true).into_bytes());
    let mut cancelled = document.begin_view(sorted_by_name()).unwrap();
    assert!(!cancelled.run(&AtomicBool::new(true)));
    assert!(!document.finish_view(cancelled));
    assert!(!document.has_view());

    let mut stale = document.begin_view(sorted_by_name()).unwrap();
    assert!(stale.run(&AtomicBool::new(false)));
    document.set_cell(1, 1, "Changed").unwrap();
    assert!(!document.finish_view(stale));
    assert!(!document.has_view());

    assert!(document.begin_view(ViewSpec {
        sort: vec![SortKey { column: 9, ascending: true }],
        filters: Vec::new()
    })
    .is_err());
}

#[test]
fn saves_over_a_file_mapped_from_disk() {
    let dir = temp_dir("mapped");
    let path = dir.join("big.csv");
    // Past the size from which files are mapped instead of read.
    let line = "1234567890,abcdefghijklmnopqrstuvwxyz,0.5\n";
    let rows = (64 << 20) / line.len() + 10;
    let mut text = String::from("id,text,value\n");
    for _ in 0..rows {
        text.push_str(line);
    }
    std::fs::write(&path, &text).unwrap();

    let mut document = Document::open(&path).unwrap();
    document.set_cell(1, 1, "edited").unwrap();
    document.save(&path).unwrap();
    // The mapped bytes are still readable after the file was replaced.
    assert_eq!(document.row(2).unwrap()[1], "abcdefghijklmnopqrstuvwxyz");

    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.row(1).unwrap()[1], "edited");
    assert_eq!(reopened.row_count(), rows + 1);
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|entry| entry.ok()).collect();
    assert_eq!(leftovers.len(), 1, "no temporary file is left behind");
    std::fs::remove_dir_all(dir).unwrap();
}
