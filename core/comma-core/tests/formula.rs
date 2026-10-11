use comma_core::{Document, Error, FormulaErrorKind};

fn open(bytes: &str) -> Document {
    Document::from_bytes(bytes.as_bytes().to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

/// The formula's value for the first data row.
fn first(document: &Document, formula: &str) -> String {
    document.preview_formula(formula, 0..1).unwrap().remove(0).unwrap()
}

fn compile_error(document: &Document, formula: &str) -> FormulaErrorKind {
    match document.compile_formula(formula) {
        Err(Error::Formula(error)) => error.kind,
        other => panic!("expected an error for {formula}, got {other:?}"),
    }
}

const ORDERS: &str = "item,price,qty,date,note\nTea,\"1 250,50\",2,2024-03-15,  hello \nCoffee,3.5,,31.01.2023,x\n";

#[test]
fn arithmetic_and_precedence() {
    let document = open(ORDERS);
    assert_eq!(first(&document, "{qty} * 3 + 1"), "7");
    assert_eq!(first(&document, "1 + 2 * 3"), "7");
    assert_eq!(first(&document, "(1 + 2) * 3"), "9");
    assert_eq!(first(&document, "10 - 4 - 3"), "3");
    assert_eq!(first(&document, "-{qty} + 5"), "3");
    assert_eq!(first(&document, "0.1 + 0.2"), "0.3");
    assert_eq!(first(&document, "7 % 3"), "1");
    assert_eq!(first(&document, "1 / 3"), "0.3333333333");
}

#[test]
fn numbers_as_people_write_them() {
    let document = open(ORDERS);
    // The price has a decimal comma, so the result keeps one.
    assert_eq!(first(&document, "{price} * {qty}"), "2501");
    assert_eq!(first(&document, "{price} / 2"), "625,25");
    // An empty value counts as zero.
    let rows = document.preview_formula("{price} * {qty}", 0..2).unwrap();
    assert_eq!(rows[1].as_ref().unwrap(), "0");
}

#[test]
fn text_functions_and_concatenation() {
    let document = open(ORDERS);
    assert_eq!(first(&document, "{item} & \" x\" & {qty}"), "Tea x2");
    assert_eq!(first(&document, "UPPER({item})"), "TEA");
    assert_eq!(first(&document, "TRIM({note})"), "hello");
    assert_eq!(first(&document, "LEN(TRIM({note}))"), "5");
    assert_eq!(first(&document, "LEFT({item}, 2) & RIGHT({item}, 1) & MID(\"abcdef\", 2, 3)"), "Teabcd");
    assert_eq!(first(&document, "REPLACE({item}, \"T\", \"t\")"), "tea");
    assert_eq!(first(&document, "CONCAT({item}, \"-\", {qty})"), "Tea-2");
    assert_eq!(first(&document, "\"say \"\"hi\"\"\""), "say \"hi\"");
    let rows = document.preview_formula("COALESCE({qty}, \"-\")", 0..2).unwrap();
    assert_eq!(rows[0].as_ref().unwrap(), "2");
    assert_eq!(rows[1].as_ref().unwrap(), "-");
}

#[test]
fn conditions() {
    let document = open(ORDERS);
    assert_eq!(first(&document, "IF({qty} > 1, \"many\", \"one\")"), "many");
    assert_eq!(first(&document, "IF({item} = \"tea\", 1, 0)"), "1");
    assert_eq!(first(&document, "IF(CONTAINS({item}, \"E\") AND NOT ISEMPTY({qty}), \"yes\", \"no\")"), "yes");
    assert_eq!(first(&document, "{qty} >= 2 OR 1 / 0 > 1"), "true");
    let rows = document.preview_formula("IF(ISEMPTY({qty}), \"none\", {qty})", 0..2).unwrap();
    assert_eq!(rows[1].as_ref().unwrap(), "none");
    assert_eq!(first(&document, "ROUND({price}, -2)"), "1300");
    assert_eq!(first(&document, "MAX({qty}, 5, 1) + MIN(3, {qty}) + ABS(-1) + SUM(1, 2)"), "11");
}

#[test]
fn dates() {
    let document = open(ORDERS);
    assert_eq!(first(&document, "YEAR({date}) & \"-\" & MONTH({date}) & \"-\" & DAY({date})"), "2024-3-15");
    let rows = document.preview_formula("DAYS({date}, \"2023-01-01\")", 0..2).unwrap();
    assert_eq!(rows[0].as_ref().unwrap(), "439");
    assert_eq!(rows[1].as_ref().unwrap(), "30");
    assert_eq!(first(&document, "{date} > \"2024-01-01\""), "true");
}

#[test]
fn columns_by_name_letter_or_number() {
    let document = open(ORDERS);
    assert_eq!(first(&document, "{Item} & {B} & {#3}"), "Tea1 250,502");
    assert_eq!(compile_error(&document, "{missing}"), FormulaErrorKind::UnknownColumn);
}

#[test]
fn syntax_errors() {
    let document = open(ORDERS);
    assert_eq!(compile_error(&document, "{qty} *"), FormulaErrorKind::UnexpectedEnd);
    assert_eq!(compile_error(&document, "{qty"), FormulaErrorKind::UnclosedColumn);
    assert_eq!(compile_error(&document, "\"open"), FormulaErrorKind::UnclosedText);
    assert_eq!(compile_error(&document, "FOO(1)"), FormulaErrorKind::UnknownFunction);
    assert_eq!(compile_error(&document, "IF(1, 2)"), FormulaErrorKind::WrongArgumentCount);
    assert_eq!(compile_error(&document, "1 2"), FormulaErrorKind::UnexpectedText);
    assert_eq!(compile_error(&document, "(1 + 2"), FormulaErrorKind::UnexpectedEnd);
    assert_eq!(compile_error(&document, "1 + 2)"), FormulaErrorKind::UnexpectedText);
    // A separator out of place is named as it was typed.
    for (formula, separator) in [("{qty}; 0", ";"), ("{qty}, 0", ",")] {
        match document.compile_formula(formula) {
            Err(Error::Formula(error)) => assert_eq!(error.detail, separator, "{formula}"),
            other => panic!("expected an error for {formula}, got {other:?}"),
        }
    }
}

#[test]
fn runtime_errors_per_row() {
    let document = open(ORDERS);
    let rows = document.preview_formula("{item} * 2", 0..1).unwrap();
    assert_eq!(rows[0].as_ref().unwrap_err().kind, FormulaErrorKind::NotANumber);
    let rows = document.preview_formula("1 / {qty}", 1..2).unwrap();
    assert_eq!(rows[0].as_ref().unwrap_err().kind, FormulaErrorKind::DivisionByZero);
    let rows = document.preview_formula("YEAR({item})", 0..1).unwrap();
    assert_eq!(rows[0].as_ref().unwrap_err().kind, FormulaErrorKind::NotADate);
}

#[test]
fn adds_a_computed_column() {
    let mut document = open("item,price,qty\nTea,10,2\nCoffee,x,3\nShort\n");
    let report = document.add_formula_column("{price} * {qty}", "total", 3).unwrap();
    assert_eq!(report.rows, 3);
    assert_eq!(report.errors, 1);
    assert_eq!(report.first_error.as_ref().unwrap().0, 2);
    assert_eq!(saved(&document), "item,price,qty,total\nTea,10,2,20\nCoffee,x,3,\nShort,,,0\n");
    assert_eq!(document.column_count(), 4);
    document.undo();
    assert_eq!(saved(&document), "item,price,qty\nTea,10,2\nCoffee,x,3\nShort\n");
    assert_eq!(document.column_count(), 3);

    document.add_formula_column("UPPER({item})", "Item, upper", 1).unwrap();
    assert_eq!(document.row(0).unwrap()[1], "Item, upper");
    assert!(saved(&document).starts_with("item,\"Item, upper\",price,qty\nTea,TEA,10,2\n"));
}

#[test]
fn fills_a_column_using_its_own_values() {
    let mut document = open("item,price\nTea,10\nCoffee,20\n");
    let report = document.fill_column_with_formula("{price} * 1.5", 1).unwrap();
    assert_eq!(report.errors, 0);
    assert_eq!(saved(&document), "item,price\nTea,15\nCoffee,30\n");
    document.undo();
    assert_eq!(saved(&document), "item,price\nTea,10\nCoffee,20\n");
}
