use comma_core::{
    Action, CellRange, Document, Edit, Filter, FilterOp, Progress, Recipe, RecipeStep, SearchQuery, SortKey,
    TextTransform, ViewSpec,
};

fn open(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec())
}

fn saved(document: &Document) -> String {
    String::from_utf8(document.to_bytes().unwrap()).unwrap()
}

/// Makes `edit` and returns the step that repeats it.
fn record(document: &mut Document, edit: Edit) -> RecipeStep {
    let step = document.recipe_step_for_edit(&edit).expect("a repeatable edit");
    document.apply_edit(&edit).unwrap();
    step
}

fn column(document: &Document, column: usize) -> CellRange {
    CellRange { top: 0, left: column, bottom: document.visible_row_count() - 1, right: column }
}

const MARCH: &str = "id,city,country,price\n1, oslo ,NO,10\n2,moscow,RU,20\n2,moscow,RU,20\n3,kazan,RU,\n";
/// The next export: columns in another order, other rows.
const APRIL: &str = "country,price,id,city\nRU,5,7,  samara\nNO,,8,bergen \nRU,5,7,  samara\n";

fn recorded_recipe() -> (Recipe, String) {
    let mut document = open(MARCH);
    let mut steps = Vec::new();
    let range = column(&document, 1);
    steps.push(record(&mut document, Edit::Transform { range, transform: TextTransform::Trim }));
    // Upper case only where the filter shows Russian cities.
    document
        .set_view(ViewSpec {
            filters: vec![Filter { column: Some(2), op: FilterOp::Equals, value: "RU".into(), case_sensitive: true }],
            sort: Vec::new(),
        })
        .unwrap();
    let range = column(&document, 1);
    steps.push(record(&mut document, Edit::Transform { range, transform: TextTransform::Uppercase }));
    document.set_view(ViewSpec::default()).unwrap();
    let range = column(&document, 3);
    steps.push(record(&mut document, Edit::FillEmpty { range, from_above: false, value: "0".into() }));
    steps.push(record(&mut document, Edit::RemoveDuplicates { columns: vec![], normalize: false }));
    steps.push(record(&mut document, Edit::AddFormulaColumn { formula: "{price} * 2".into(), name: "double".into(), at: 4 }));
    steps.push(document.recipe_step_for_move_column(2, 0));
    document.move_column(2, 0).unwrap();
    steps.push(document.recipe_step_for_set_cell(0, 1, "ID").unwrap());
    document.set_cell(0, 1, "ID").unwrap();
    (Recipe { name: "Cleanup".into(), steps }, saved(&document))
}

#[test]
fn recorded_steps_repeat_on_the_same_file() {
    let (recipe, expected) = recorded_recipe();
    assert_eq!(expected, "country,ID,city,price,double\nNO,1,oslo,10,20\nRU,2,MOSCOW,20,40\nRU,3,KAZAN,0,0\n");
    let mut document = open(MARCH);
    let outcome = document.apply_edit(&Edit::Recipe(recipe)).unwrap();
    assert!(outcome.changed);
    assert_eq!(saved(&document), expected);
    // One undoable step.
    assert!(document.undo());
    assert_eq!(saved(&document), MARCH);
    assert!(!document.can_undo());
}

#[test]
fn recipes_find_columns_by_name_in_the_next_export() {
    let (recipe, _) = recorded_recipe();
    let recipe = Recipe::from_json(&recipe.to_json()).unwrap();
    let mut document = open(APRIL);
    document.apply_edit(&Edit::Recipe(recipe)).unwrap();
    assert_eq!(saved(&document), "country,price,double,ID,city\nRU,5,10,7,SAMARA\nNO,0,0,8,bergen\n");
}

#[test]
fn json_names_columns_and_keeps_filters() {
    let (recipe, _) = recorded_recipe();
    let json = recipe.to_json();
    assert!(json.contains("\"action\": \"transform\""), "{json}");
    assert!(json.contains("\"columns\": [\n        \"city\"\n      ]"), "{json}");
    assert!(json.contains("\"filters\""), "{json}");
    assert_eq!(Recipe::from_json(&json).unwrap(), recipe);
    assert!(Recipe::from_json("{\"steps\": [{\"action\": \"explode\"}]}").is_err());
}

#[test]
fn a_failing_step_names_itself_and_changes_nothing() {
    let recipe = Recipe {
        name: String::new(),
        steps: vec![
            RecipeStep { action: Action::Transform { columns: vec!["city".into()], transform: TextTransform::Uppercase }, rows: None },
            RecipeStep { action: Action::DeleteColumn { column: "missing".into() }, rows: None },
        ],
    };
    let mut document = open(MARCH);
    let error = document.apply_edit(&Edit::Recipe(recipe)).unwrap_err();
    assert_eq!(error.to_string(), "step 2: there is no column “missing”");
    assert_eq!(saved(&document), MARCH);
    assert!(!document.can_undo());
}

#[test]
fn deleting_the_filtered_rows_repeats_as_a_filter() {
    let mut document = open(MARCH);
    document
        .set_view(ViewSpec {
            filters: vec![Filter { column: Some(2), op: FilterOp::Equals, value: "RU".into(), case_sensitive: false }],
            sort: vec![SortKey { column: 0, ascending: false }],
        })
        .unwrap();
    let rows: Vec<usize> = (0..document.visible_row_count()).filter_map(|row| document.visible_record(row)).collect();
    let step = document.recipe_step_for_delete_rows(&rows).unwrap();
    assert!(document.recipe_step_for_delete_rows(&rows[..1]).is_none());
    let recipe = Recipe { name: String::new(), steps: vec![step] };
    let mut other = open(APRIL);
    other.apply_edit(&Edit::Recipe(recipe)).unwrap();
    assert_eq!(saved(&other), "country,price,id,city\nNO,,8,bergen \n");
}

#[test]
fn edits_of_some_rows_are_not_recorded() {
    let document = open(MARCH);
    let some = CellRange { top: 1, left: 1, bottom: 2, right: 1 };
    assert!(document.recipe_step_for_edit(&Edit::Transform { range: some, transform: TextTransform::Uppercase }).is_none());
    let query = SearchQuery { text: "o".into(), range: Some(some), ..SearchQuery::default() };
    assert!(document.recipe_step_for_edit(&Edit::ReplaceAll { query, replacement: "0".into() }).is_none());
    assert!(document.recipe_step_for_set_cell(1, 1, "x").is_none());
}

#[test]
fn sorting_and_moving_columns_repeat() {
    let mut document = open("b,a,c\n2,x,1\n1,y,2\n");
    let mut steps = vec![document.recipe_step_for_move_column(0, 2)];
    document.move_column(0, 2).unwrap();
    document.set_view(ViewSpec { sort: vec![SortKey { column: 2, ascending: true }], filters: Vec::new() }).unwrap();
    steps.push(document.recipe_step_for_apply_sort().unwrap());
    document.apply_sort().unwrap();
    let expected = saved(&document);
    assert_eq!(expected, "a,c,b\ny,2,1\nx,1,2\n");
    let mut again = open("b,a,c\n2,x,1\n1,y,2\n");
    again.apply_edit(&Edit::Recipe(Recipe { name: String::new(), steps })).unwrap();
    assert_eq!(saved(&again), expected);
}

#[test]
fn background_recipe_reports_progress_and_cancels() {
    let (recipe, expected) = recorded_recipe();
    let mut document = open(MARCH);
    let mut task = document.begin_edit(Edit::Recipe(recipe.clone()));
    let progress = Progress::default();
    assert!(task.run(&progress));
    assert_eq!(progress.fraction(), 1.0);
    document.finish_edit(task).unwrap().unwrap();
    assert_eq!(saved(&document), expected);

    let mut document = open(MARCH);
    let mut task = document.begin_edit(Edit::Recipe(recipe));
    let progress = Progress::default();
    progress.cancel();
    assert!(!task.run(&progress));
    assert_eq!(document.finish_edit(task).unwrap(), None);
    assert_eq!(saved(&document), MARCH);
}

/// The pandas script makes the same file, where Python with pandas is installed.
#[test]
fn pandas_script_makes_the_same_file() {
    let has_pandas = std::process::Command::new("python3")
        .args(["-c", "import pandas"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !has_pandas {
        eprintln!("python3 with pandas not found: skipped");
        return;
    }
    let (recipe, _) = recorded_recipe();
    let dir = std::env::temp_dir().join(format!("comma-recipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (input, output) = (dir.join("april.csv"), dir.join("out.csv"));
    std::fs::write(&input, APRIL).unwrap();
    let script = recipe.to_pandas(input.to_str().unwrap(), output.to_str().unwrap(), ',');
    std::fs::write(dir.join("recipe.py"), &script).unwrap();
    let run = std::process::Command::new("python3").arg(dir.join("recipe.py")).output().unwrap();
    assert!(run.status.success(), "{}\n{script}", String::from_utf8_lossy(&run.stderr));
    let mut document = open(APRIL);
    document.apply_edit(&Edit::Recipe(recipe)).unwrap();
    assert_eq!(std::fs::read_to_string(&output).unwrap(), saved(&document), "{script}");
    std::fs::remove_dir_all(dir).ok();
}
