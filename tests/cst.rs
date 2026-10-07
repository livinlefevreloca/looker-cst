//! Parser structure, round-trip and editing tests on hand written LookML.

use std::fs;
use std::path::Path;

use albert_looker_cst::edit::{block_value, escape_string, infer_scalar, new_pair, unescape_string};
use albert_looker_cst::{ListValue, PairRef, Value, parse};

const VIEW: &str = "\
# users view
view: users {
  sql_table_name: public.users ;;

  # the id
  dimension: id {
    primary_key: yes
    type: number
    sql: ${TABLE}.id ;;
  }

  dimension: name {
    type: string
    sql: ${TABLE}.name ;;
  }
}
";

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn child(parent: &PairRef, key: &str, name: Option<&str>) -> PairRef {
    parent.read().body().unwrap().find(key, name).unwrap()
}

#[test]
fn fixtures_round_trip() {
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")).unwrap() {
        let path = entry.unwrap().path();
        let source = fs::read_to_string(&path).unwrap();
        let doc = parse(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(doc.to_string(), source, "{}", path.display());
    }
}

#[test]
fn parses_structure_of_edge_cases() {
    let doc = parse(&fixture("edge_cases.view.lkml")).unwrap();
    let keys: Vec<String> = doc.body.items.iter().map(|p| p.read().key.clone()).collect();
    assert_eq!(keys, ["include", "view", "explore", "explore", "datagroup"]);

    let view = doc.body.find("view", None).unwrap();
    assert_eq!(view.read().name(), Some("+users"));

    let table = child(&view, "sql_table_name", None);
    assert_eq!(table.read().text().unwrap(), "analytics.users");

    let quoted = child(&view, "dimension", Some("quoted"));
    let label = child(&quoted, "label", None);
    assert_eq!(label.read().text().unwrap(), r#"Has "quotes" and a \ backslash"#);
    let sql = child(&quoted, "sql", None).read().text().unwrap();
    assert!(sql.starts_with("CASE") && sql.ends_with("END"), "{sql}");
    assert!(sql.contains("'a;b'"));

    let status = child(&view, "dimension", Some("status"));
    let case = child(&status, "case", None);
    assert!(case.read().name().is_none());
    assert_eq!(child(&case, "else", None).read().text().unwrap(), "Inactive");

    let created = child(&view, "dimension_group", Some("created"));
    let timeframes = child(&created, "timeframes", None);
    let Value::List(list) = &timeframes.read().value else {
        panic!("expected a list")
    };
    let elements: Vec<String> = list.items.iter().map(|i| i.value.to_string()).collect();
    assert_eq!(elements, ["raw", "date", "week", "month"]);
    assert!(list.items[3].comma.is_some(), "trailing comma is kept");

    let total = child(&view, "measure", Some("total"));
    let filters = child(&total, "filters", None);
    let Value::List(list) = &filters.read().value else {
        panic!("expected a list")
    };
    let ListValue::Pair { key, value, .. } = &list.items[1].value else {
        panic!("expected a pair")
    };
    assert_eq!(key, "created_date");
    assert!(matches!(value.as_ref(), ListValue::String(s) if s == "last 30 days"));

    let spaced = child(&view, "filter", Some("spaced_colon"));
    assert_eq!(spaced.read().before_colon, " ");
}

#[test]
fn reports_error_positions() {
    let err = parse("view: users {\n  dimension: id {\n    sql: ${TABLE}.id\n  }\n}\n").unwrap_err();
    assert!(err.message.contains(";;"), "{err}");
    assert_eq!((err.line, err.column), (3, 9));

    let err = parse("view: users {\n  dimension: id {\n").unwrap_err();
    assert!(err.message.contains('}'), "{err}");
    assert_eq!(err.line, 3);

    let err = parse("view: users {\n  label: \"oops\n}\n").unwrap_err();
    assert_eq!(err.message, "unterminated string");

    let err = parse("view: users {\n  fields: [a b]\n}\n").unwrap_err();
    assert!(err.message.contains("list"), "{err}");

    assert!(parse("}").is_err());
    assert!(parse("view users {}").is_err());
}

#[test]
fn set_text_keeps_kind_and_spacing() {
    let doc = parse(VIEW).unwrap();
    let view = doc.body.find("view", None).unwrap();
    let id = child(&view, "dimension", Some("id"));
    child(&id, "type", None).write().set_text("string");
    child(&id, "sql", None).write().set_text("${TABLE}.user_id");
    assert_eq!(
        doc.to_string(),
        VIEW.replace("type: number", "type: string")
            .replace("${TABLE}.id", "${TABLE}.user_id")
    );
}

#[test]
fn string_escaping_round_trips() {
    for text in [r#"say "hi""#, r"back\slash", r"ends\", r"\d+", r#"\""#, ""] {
        assert_eq!(unescape_string(&escape_string(text)), text, "{text:?}");
    }
    assert_eq!(escape_string(r"\d"), r"\d", "lone escapes are left as written");
}

#[test]
fn infers_value_kinds() {
    assert!(matches!(infer_scalar("type", "number"), Value::Literal(_)));
    assert!(matches!(infer_scalar("label", "Id"), Value::String(_)));
    assert!(matches!(infer_scalar("type", "two words"), Value::String(_)));
    assert!(matches!(infer_scalar("sql", "${TABLE}.id"), Value::Expr(_)));
    assert!(matches!(infer_scalar("sql_on", "a = b"), Value::Expr(_)));
}

#[test]
fn appended_pairs_follow_sibling_spacing() {
    let doc = parse(VIEW).unwrap();
    let view = doc.body.find("view", None).unwrap();
    let indent = "";
    let email = view.write().body_mut().unwrap().insert(
        usize::MAX,
        new_pair("dimension", block_value(Some("email"))),
        indent,
        false,
    );
    email.write().body_mut().unwrap().insert(
        0,
        new_pair("type", infer_scalar("type", "string")),
        "  ",
        false,
    );

    let expected = VIEW.replace(
        "    sql: ${TABLE}.name ;;\n  }\n}",
        "    sql: ${TABLE}.name ;;\n  }\n\n  dimension: email {\n    type: string\n  }\n}",
    );
    assert_eq!(doc.to_string(), expected);
}

#[test]
fn inserting_at_top_keeps_blank_line_style() {
    let doc = parse(VIEW).unwrap();
    let view = doc.body.find("view", None).unwrap();
    view.write()
        .body_mut()
        .unwrap()
        .insert(0, new_pair("label", infer_scalar("label", "Users")), "", false);
    let expected = VIEW.replace(
        "view: users {\n  sql_table_name",
        "view: users {\n  label: \"Users\"\n\n  sql_table_name",
    );
    assert_eq!(doc.to_string(), expected);
}

#[test]
fn removing_takes_leading_comments() {
    let mut doc = parse(VIEW).unwrap();
    let view = doc.body.find("view", None).unwrap();
    let id = child(&view, "dimension", Some("id"));
    assert!(view.write().body_mut().unwrap().remove_pair(&id, false));
    let expected = VIEW.replace(
        "  # the id\n  dimension: id {\n    primary_key: yes\n    type: number\n    sql: ${TABLE}.id ;;\n  }\n\n",
        "",
    );
    assert_eq!(doc.to_string(), expected);

    // Removing the first top level pair keeps the file header spacing for the next one.
    let explore_doc = "explore: a {}\n\nexplore: b {}\n";
    let mut explores = parse(explore_doc).unwrap();
    explores.body.remove(0, true);
    assert_eq!(explores.to_string(), "explore: b {}\n");
    doc.body.items.clear();
    assert_eq!(doc.to_string(), "\n");
}

#[test]
fn removing_only_child_collapses_block() {
    let doc = parse("explore: a {\n  label: \"A\"\n}\n").unwrap();
    let explore = doc.body.find("explore", None).unwrap();
    explore.write().body_mut().unwrap().remove(0, false);
    assert_eq!(doc.to_string(), "explore: a {}\n");
}

#[test]
fn adding_to_empty_block_opens_it() {
    let doc = parse("view: a {\n  dimension: b {}\n}\n").unwrap();
    let view = doc.body.find("view", None).unwrap();
    let b = child(&view, "dimension", Some("b"));
    b.write()
        .body_mut()
        .unwrap()
        .insert(0, new_pair("type", infer_scalar("type", "number")), "  ", false);
    assert_eq!(
        doc.to_string(),
        "view: a {\n  dimension: b {\n    type: number\n  }\n}\n"
    );
}

#[test]
fn adding_to_empty_document() {
    let mut doc = parse("").unwrap();
    doc.body.insert(
        0,
        new_pair("connection", infer_scalar("connection", "prod")),
        "",
        true,
    );
    doc.body
        .insert(1, new_pair("explore", block_value(Some("a"))), "", true);
    assert_eq!(doc.to_string(), "connection: \"prod\"\nexplore: a {}\n");
}

#[test]
fn insert_source_reindents_snippet() {
    let doc = parse(VIEW).unwrap();
    let view = doc.body.find("view", None).unwrap();
    view.write()
        .body_mut()
        .unwrap()
        .insert_source(
            usize::MAX,
            "measure: count {\n  type: count\n}\n\nmeasure: total {\n  type: sum\n}",
            "",
            false,
        )
        .unwrap();
    let expected = VIEW.replace(
        "    sql: ${TABLE}.name ;;\n  }\n}",
        "    sql: ${TABLE}.name ;;\n  }\n\n  measure: count {\n    type: count\n  }\n\n  measure: total {\n    type: sum\n  }\n}",
    );
    assert_eq!(doc.to_string(), expected);
}

#[test]
fn replacing_list_keeps_layout() {
    let source = "set: s {\n  fields: [\n    a,\n    b,\n  ]\n  other: [x, y]\n}\n";
    let doc = parse(source).unwrap();
    let set = doc.body.find("set", None).unwrap();
    let new_list = |names: &[&str]| {
        albert_looker_cst::edit::list_of(names.iter().map(|n| ListValue::Literal(n.to_string())).collect())
    };
    child(&set, "fields", None)
        .write()
        .set_value(Value::List(new_list(&["a", "b", "c"])));
    child(&set, "other", None)
        .write()
        .set_value(Value::List(new_list(&["z"])));
    assert_eq!(
        doc.to_string(),
        "set: s {\n  fields: [\n    a,\n    b,\n    c,\n  ]\n  other: [z]\n}\n"
    );
}

#[test]
fn replacing_list_keeps_leading_commas() {
    let source = "set: s {\n  fields: [\n    a\n    , b\n    ]\n}\n";
    let doc = parse(source).unwrap();
    let set = doc.body.find("set", None).unwrap();
    let elements = ["a", "b", "c"]
        .iter()
        .map(|n| ListValue::Literal(n.to_string()))
        .collect();
    child(&set, "fields", None)
        .write()
        .set_value(Value::List(albert_looker_cst::edit::list_of(elements)));
    assert_eq!(
        doc.to_string(),
        "set: s {\n  fields: [\n    a\n    , b\n    , c\n    ]\n}\n"
    );
}

#[test]
fn new_blocks_copy_spacing_of_similar_siblings() {
    let source = "view: v {\n  sql_table_name: t ;;\n  drill_fields: [id]\n\n  dimension: id {}\n}\n";
    let doc = parse(source).unwrap();
    let view = doc.body.find("view", None).unwrap();
    let mut guard = view.write();
    let body = guard.body_mut().unwrap();
    body.insert(2, new_pair("dimension", block_value(Some("email"))), "", false);
    body.insert(2, new_pair("label", infer_scalar("label", "V")), "", false);
    drop(guard);
    assert_eq!(
        doc.to_string(),
        "view: v {\n  sql_table_name: t ;;\n  drill_fields: [id]\n  label: \"V\"\n\n  dimension: email {}\n\n  dimension: id {}\n}\n"
    );
}

#[test]
fn insert_source_keeps_comments_above_first_pair() {
    let doc = parse("view: v {\n  dimension: a {}\n}\n").unwrap();
    let view = doc.body.find("view", None).unwrap();
    view.write()
        .body_mut()
        .unwrap()
        .insert_source(
            usize::MAX,
            "# why b exists\n# second line\ndimension: b {}",
            "",
            false,
        )
        .unwrap();
    assert_eq!(
        doc.to_string(),
        "view: v {\n  dimension: a {}\n  # why b exists\n  # second line\n  dimension: b {}\n}\n"
    );
}
