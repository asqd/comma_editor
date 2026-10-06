use comma_core::{Document, Filter, FilterOp, Issue, ValueKind, ViewSpec};

fn open(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec())
}

const DATA: &str = "id,amount,city,joined\n\
1,10,Moscow,2024-01-01\n\
2,20.5, Moscow ,2024-03-01\n\
3,n/a,Kazan,2024-02-01\n\
4,,Moscow,2024-02-15\n\
5,30,Paris,soon\n\
6,40,Kazan,2024-04-01\n";

#[test]
fn numbers_with_a_stray_value() {
    let profile = open(DATA).profile_column(1).unwrap();
    assert_eq!(profile.kind, ValueKind::Decimal);
    assert_eq!((profile.total, profile.empty, profile.unique), (6, 1, 5));
    assert_eq!(profile.mismatched, 1);
    assert_eq!(profile.min.as_deref(), Some("10"));
    assert_eq!(profile.max.as_deref(), Some("40"));
    assert_eq!(profile.mean, Some(25.125));
    assert_eq!(profile.median, Some(25.25));
    assert_eq!(profile.histogram.iter().sum::<usize>(), 4);
    assert_eq!(profile.histogram.len(), 20);
}

#[test]
fn text_with_padding_and_top_values() {
    let profile = open(DATA).profile_column(2).unwrap();
    assert_eq!(profile.kind, ValueKind::Text);
    assert_eq!(profile.padded, 1);
    assert_eq!(profile.mismatched, 0);
    // Ties go alphabetically; " Moscow " with spaces is a value of its own.
    assert_eq!(profile.top_values[0], ("Kazan".to_owned(), 2));
    assert_eq!(profile.top_values[1], ("Moscow".to_owned(), 2));
    assert_eq!((profile.min_length, profile.max_length), (5, 8));
    assert!(profile.histogram.is_empty());
}

#[test]
fn dates_and_issues() {
    let document = open(DATA);
    let profile = document.profile_column(3).unwrap();
    assert_eq!(profile.kind, ValueKind::Date);
    assert_eq!(profile.mismatched, 1);
    assert_eq!(profile.min.as_deref(), Some("2024-01-01"));
    assert_eq!(profile.max.as_deref(), Some("2024-04-01"));
    assert_eq!(profile.mean, None);

    assert_eq!(document.find_issue(3, Issue::Mismatch, ValueKind::Date, None), Some(4));
    assert_eq!(document.find_issue(3, Issue::Mismatch, ValueKind::Date, Some(4)), Some(4));
    assert_eq!(document.find_issue(2, Issue::Padding, ValueKind::Text, Some(3)), Some(1));
    assert_eq!(document.find_issue(0, Issue::Padding, ValueKind::Integer, None), None);
}

#[test]
fn profiles_follow_the_view() {
    let mut document = open(DATA);
    document
        .set_view(ViewSpec {
            sort: vec![],
            filters: vec![Filter { column: Some(2), op: FilterOp::Equals, value: "kazan".into(), case_sensitive: false }],
        })
        .unwrap();
    let profile = document.profile_column(1).unwrap();
    assert_eq!(profile.total, 2);
    assert_eq!(profile.kind, ValueKind::Text);
}
