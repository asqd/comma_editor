use comma_core::{DiffCell, DiffColumn, DiffEntry, DiffOptions, DiffStatus, Document, Progress};

fn open(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec())
}

fn diff(left: &str, right: &str, options: DiffOptions) -> comma_core::Diff {
    open(left).diff(&open(right), &options, &Progress::default()).unwrap().unwrap()
}

fn statuses(diff: &comma_core::Diff) -> Vec<(DiffStatus, Option<usize>, Option<usize>)> {
    diff.entries().iter().map(|e| (e.status, e.left, e.right)).collect()
}

const OLD: &str = "id,name,price\n1,Tea,10\n2,Coffee,20\n3,Milk,5\n";
const NEW: &str = "id,price,name,stock\n3,5,Milk,7\n1,12,Tea,1\n4,8,Sugar,0\n";

#[test]
fn matches_columns_by_name() {
    let columns = open(OLD).diff_columns(&open(NEW));
    assert_eq!(
        columns,
        vec![
            DiffColumn { name: Some("id".into()), left: Some(0), right: Some(0) },
            DiffColumn { name: Some("name".into()), left: Some(1), right: Some(2) },
            DiffColumn { name: Some("price".into()), left: Some(2), right: Some(1) },
            DiffColumn { name: Some("stock".into()), left: None, right: Some(3) },
        ]
    );
    assert_eq!(open(OLD).suggest_diff_key(&open(NEW)), vec![0]);
}

#[test]
fn finds_added_removed_and_changed_rows_by_key() {
    let diff = diff(OLD, NEW, DiffOptions { key: vec![0], ..DiffOptions::default() });
    assert_eq!(
        statuses(&diff),
        vec![
            (DiffStatus::Changed, Some(1), Some(2)),
            (DiffStatus::OnlyLeft, Some(2), None),
            (DiffStatus::Same, Some(3), Some(1)),
            (DiffStatus::OnlyRight, None, Some(3)),
        ]
    );
    let counts = diff.counts();
    assert_eq!((counts.only_left, counts.only_right, counts.changed, counts.same), (1, 1, 1, 1));
    let tea = diff.cells(&diff.entries()[0]);
    assert_eq!(tea[2], DiffCell { left: Some("10".into()), right: Some("12".into()), changed: true });
    assert!(!tea[1].changed);
    // A column only one table has is shown but not compared.
    assert_eq!(tea[3], DiffCell { left: None, right: Some("1".into()), changed: false });

    let csv = String::from_utf8(diff.to_csv(
        &[DiffStatus::OnlyLeft, DiffStatus::OnlyRight, DiffStatus::Changed],
        ["removed", "added", "changed", "same"],
        "status",
    ))
    .unwrap();
    assert_eq!(
        csv,
        "status,id,name,price,stock\nchanged,1,Tea,10 → 12,1\nremoved,2,Coffee,20,\nadded,4,Sugar,8,0\n"
    );
}

#[test]
fn compares_by_position_without_a_key_or_header() {
    let diff = diff("a,1\nb,2\nc,3\n", "a,1\nb,9\n", DiffOptions::default());
    assert_eq!(
        statuses(&diff),
        vec![
            (DiffStatus::Same, Some(0), Some(0)),
            (DiffStatus::Changed, Some(1), Some(1)),
            (DiffStatus::OnlyLeft, Some(2), None),
        ]
    );
}

#[test]
fn ignores_case_and_spaces_when_asked() {
    let left = "code,name\nA1,Tea\n";
    let right = "code,name\na1 , tea\n";
    let strict = diff(left, right, DiffOptions { key: vec![0], ..DiffOptions::default() });
    assert_eq!(strict.counts().only_left, 1);
    let loose = diff(left, right, DiffOptions { key: vec![0], ignore_case: true, trim: true });
    assert_eq!(statuses(&loose), vec![(DiffStatus::Same, Some(1), Some(1))]);
}

#[test]
fn pairs_repeated_keys_in_order() {
    let diff = diff("k,v\nx,1\nx,2\ny,3\n", "k,v\nx,1\nx,5\nx,6\n", DiffOptions { key: vec![0], ..DiffOptions::default() });
    assert_eq!(
        statuses(&diff),
        vec![
            (DiffStatus::Same, Some(1), Some(1)),
            (DiffStatus::Changed, Some(2), Some(2)),
            (DiffStatus::OnlyLeft, Some(3), None),
            (DiffStatus::OnlyRight, None, Some(3)),
        ]
    );
    assert_eq!((diff.counts().repeated_left, diff.counts().repeated_right), (1, 2));
}

#[test]
fn large_tables_compare_like_small_ones() {
    let mut left = String::from("id,v\n");
    let mut right = String::from("id,v\n");
    for n in 0..50_000 {
        left.push_str(&format!("{n},{}\n", n % 10));
        if n % 5 != 0 {
            right.push_str(&format!("{n},{}\n", if n % 7 == 0 { 99 } else { n % 10 }));
        }
    }
    let diff = diff(&left, &right, DiffOptions { key: vec![0], ..DiffOptions::default() });
    let counts = diff.counts();
    assert_eq!(counts.only_left, 10_000);
    assert_eq!(counts.only_right, 0);
    let changed = (0..50_000).filter(|n| n % 5 != 0 && n % 7 == 0).count();
    assert_eq!(counts.changed, changed);
    assert_eq!(counts.same, 40_000 - changed);
    let first: Vec<&DiffEntry> = diff.entries().iter().take(2).collect();
    assert_eq!(first[0].status, DiffStatus::OnlyLeft);
    assert_eq!(first[1].left, Some(2));
}

#[test]
fn cancelled_comparison_returns_nothing() {
    let progress = Progress::default();
    progress.cancel();
    let result = open(OLD).diff(&open(NEW), &DiffOptions { key: vec![0], ..DiffOptions::default() }, &progress).unwrap();
    assert!(result.is_none());
}
