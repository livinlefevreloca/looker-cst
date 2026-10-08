"""Tests for the Python API: reading, editing and rewriting LookML."""

from pathlib import Path

import pytest

import looker_cst
from looker_cst import Document, LookmlSyntaxError, Pair, parse

FIXTURES = Path(__file__).parent.parent / "fixtures"

VIEW = """\
# users view
view: users {
  sql_table_name: public.users ;;

  # the id
  dimension: id {
    primary_key: yes
    type: number
    sql: ${TABLE}.id ;;
  }

  measure: count {
    type: count
    drill_fields: [id, name]
    filters: [name: "-NULL"]
  }
}
"""


@pytest.fixture
def doc() -> Document:
    return parse(VIEW)


@pytest.fixture
def view(doc: Document) -> Pair:
    return doc.find("view", "users")


@pytest.mark.parametrize("path", sorted(FIXTURES.iterdir()), ids=lambda p: p.name)
def test_fixtures_round_trip(path: Path) -> None:
    source = path.read_bytes().decode()
    assert str(parse(source)) == source


def test_document_parse_alias() -> None:
    assert str(Document.parse(VIEW)) == VIEW
    assert looker_cst.parse_lookml is looker_cst.parse


def test_reading_values(view: Pair) -> None:
    dim = view.find("dimension", "id")
    assert (dim.key, dim.name, dim.kind) == ("dimension", "id", "block")
    assert dim["type"] == "number"
    assert dim["sql"] == "${TABLE}.id"
    assert dim.find("sql").kind == "expr"
    assert dim.find("primary_key").kind == "literal"
    assert dim.value is None
    measure = view.find("measure", "count")
    assert measure["drill_fields"] == ["id", "name"]
    assert measure["filters"] == [("name", "-NULL")]
    assert measure.get("missing", "default") == "default"
    assert "type" in measure and "label" not in measure
    assert [p.key for p in measure] == ["type", "drill_fields", "filters"]
    assert len(measure) == 3
    with pytest.raises(KeyError):
        measure["label"]


def test_comments_and_walk(doc: Document, view: Pair) -> None:
    assert view.find("dimension", "id").comments == ["# the id"]
    assert doc.find("view").comments == ["# users view"]
    keys = [p.key for p in doc.walk()]
    assert keys[:4] == ["view", "sql_table_name", "dimension", "primary_key"]
    assert len(keys) == 10


def test_set_scalars_keep_kind(doc: Document, view: Pair) -> None:
    dim = view.find("dimension", "id")
    dim["type"] = "string"
    dim["sql"] = "${TABLE}.user_id"
    dim["primary_key"] = False
    assert str(doc) == (
        VIEW.replace("type: number", "type: string")
        .replace("${TABLE}.id", "${TABLE}.user_id")
        .replace("primary_key: yes", "primary_key: no")
    )


def test_literal_needing_quotes_becomes_string(view: Pair) -> None:
    dim = view.find("dimension", "id")
    dim["type"] = "two words"
    assert dim.find("type").to_string() == 'type: "two words"'


def test_string_escaping(view: Pair) -> None:
    dim = view.find("dimension", "id")
    pair = dim.add("label", 'Say "hi" \\ there')
    # A backslash only needs doubling where it would escape a quote or another backslash.
    assert pair.to_string() == 'label: "Say \\"hi\\" \\ there"'
    dim.add("description", 'ends with \\')
    assert dim.find("description").to_string() == 'description: "ends with \\\\"'
    reparsed = parse(str(view.parent)).find("view").find("dimension", "id")
    assert reparsed["label"] == 'Say "hi" \\ there'


def test_add_infers_kinds_and_spacing(doc: Document, view: Pair) -> None:
    dim = view.add("dimension", name="email", index=2)
    assert dim.add("type", "string").kind == "literal"
    assert dim.add("label", "Email").kind == "string"
    assert dim.add("sql", "${TABLE}.email").kind == "expr"
    assert dim.add("hidden", True).to_string() == "hidden: yes"
    assert dim.add("tags", ["pii", "contact info"]).to_string() == 'tags: [pii, "contact info"]'
    assert dim.add("value_format_name", "usd", kind="string").to_string() == 'value_format_name: "usd"'
    expected = VIEW.replace(
        "  measure: count {",
        "  dimension: email {\n"
        "    type: string\n"
        '    label: "Email"\n'
        "    sql: ${TABLE}.email ;;\n"
        "    hidden: yes\n"
        '    tags: [pii, "contact info"]\n'
        '    value_format_name: "usd"\n'
        "  }\n"
        "\n"
        "  measure: count {",
    )
    assert str(doc) == expected


def test_add_at_top_and_negative_index(doc: Document, view: Pair) -> None:
    view.add("label", "Users", index=0)
    measure = view.find("measure", "count")
    measure.add("description", "All users", index=-1)
    assert [p.key for p in measure] == ["type", "drill_fields", "description", "filters"]
    assert str(doc).startswith('# users view\nview: users {\n  label: "Users"\n\n  sql_table_name')


def test_setitem_adds_missing_pair(view: Pair) -> None:
    measure = view.find("measure", "count")
    measure["label"] = "Count"
    assert measure.children[-1].to_string() == 'label: "Count"'


def test_lists_keep_layout_and_quoting(doc: Document, view: Pair) -> None:
    measure = view.find("measure", "count")
    measure["drill_fields"] = ["id", "name", "email"]
    measure["filters"] = {"name": "-NULL", "email": "%@albert.com"}
    assert measure.find("drill_fields").to_string() == "drill_fields: [id, name, email]"
    assert measure.find("filters").to_string() == 'filters: [name: "-NULL", email: "%@albert.com"]'
    assert measure["filters"] == [("name", "-NULL"), ("email", "%@albert.com")]

    multi = parse("set: s {\n  fields: [\n    a,\n    b,\n  ]\n}\n")
    multi.find("set")["fields"] = ["a", "b", "c"]
    assert str(multi) == "set: s {\n  fields: [\n    a,\n    b,\n    c,\n  ]\n}\n"


def test_set_value_changes_kind(view: Pair) -> None:
    dim = view.find("dimension", "id")
    dim.find("type").set_value("number", kind="string")
    assert dim.find("type").to_string() == 'type: "number"'
    with pytest.raises(TypeError):
        dim["type"] = ["a"]
    with pytest.raises(ValueError):
        dim.find("sql").set_value("a ;; b", kind="expr")
    with pytest.raises(ValueError):
        dim.find("type").set_value("has space", kind="literal")
    with pytest.raises(ValueError):
        dim.find("type").set_value("x", kind="nonsense")


def test_blocks_cannot_take_scalars(view: Pair) -> None:
    with pytest.raises(TypeError):
        view["dimension"] = "x"
    with pytest.raises(TypeError):
        view.find("sql_table_name").children


def test_remove_and_detach(doc: Document, view: Pair) -> None:
    assert view.remove("dimension", name="id")
    assert not view.remove("dimension", name="id")
    assert str(doc) == VIEW.replace(
        "  # the id\n  dimension: id {\n    primary_key: yes\n    type: number\n    sql: ${TABLE}.id ;;\n  }\n\n",
        "",
    )
    measure = view.find("measure", "count")
    filters = measure.find("filters")
    assert filters.detach()
    assert not filters.detach()
    assert "filters" not in measure
    del measure["drill_fields"]
    with pytest.raises(KeyError):
        del measure["drill_fields"]
    assert view.remove(measure)
    assert str(doc) == "# users view\nview: users {\n  sql_table_name: public.users ;;\n}\n"


def test_removing_last_child_collapses_block() -> None:
    doc = parse("explore: a {\n  label: \"A\"\n}\n")
    del doc.find("explore")["label"]
    assert str(doc) == "explore: a {}\n"
    doc.find("explore")["label"] = "B"
    assert str(doc) == 'explore: a {\n  label: "B"\n}\n'


def test_rename_key_and_name(doc: Document, view: Pair) -> None:
    view.name = "+users"
    dim = view.find("dimension", "id")
    dim.name = "user_id"
    dim.find("type").key = "label"
    assert view.find("dimension", "user_id") == dim
    assert str(doc).startswith("# users view\nview: +users {")
    with pytest.raises(ValueError):
        dim.find("label").key = "sql"
    with pytest.raises(ValueError):
        dim.name = "bad name"


def test_set_comments(doc: Document, view: Pair) -> None:
    dim = view.find("dimension", "id")
    dim.comments = ["primary key", "# used everywhere"]
    assert "\n\n  # primary key\n  # used everywhere\n  dimension: id {" in str(doc)
    dim.comments = []
    assert "\n\n  dimension: id {" in str(doc)


def test_add_source_reindents(doc: Document, view: Pair) -> None:
    added = view.add_source("dimension: email {\n  type: string\n}\n\n# total\nmeasure: total {\n  type: sum\n}")
    assert [(p.key, p.name) for p in added] == [("dimension", "email"), ("measure", "total")]
    assert added[1].comments == ["# total"]
    assert str(doc).endswith(
        "  dimension: email {\n    type: string\n  }\n\n  # total\n  measure: total {\n    type: sum\n  }\n}\n"
    )
    with pytest.raises(LookmlSyntaxError):
        view.add_source("dimension: {")


def test_handles_are_live_and_hashable(doc: Document) -> None:
    first = doc.find("view")
    second = doc.children[0]
    assert first == second and hash(first) == hash(second)
    assert {first, second} == {first}
    second["label"] = "Users"
    assert first["label"] == "Users"
    parent = first.find("dimension", "id").parent
    assert parent == first
    assert isinstance(first.parent, Document)


def test_new_document_from_scratch() -> None:
    doc = parse("")
    doc["connection"] = "redshift_prod"
    doc.add("include", "/views/*.view")
    explore = doc.add("explore", name="users")
    join = explore.add("join", name="orders")
    join["type"] = "left_outer"
    join["sql_on"] = "${users.id} = ${orders.user_id}"
    assert str(doc) == (
        'connection: "redshift_prod"\n'
        'include: "/views/*.view"\n'
        "\n"
        "explore: users {\n"
        "  join: orders {\n"
        "    type: left_outer\n"
        "    sql_on: ${users.id} = ${orders.user_id} ;;\n"
        "  }\n"
        "}\n"
    )
    assert str(parse(str(doc))) == str(doc)


def test_syntax_errors_report_position() -> None:
    with pytest.raises(LookmlSyntaxError) as info:
        parse("view: users {\n  dimension: id {\n    sql: ${TABLE}.id\n  }\n}\n")
    assert (info.value.line, info.value.column) == (3, 9)
    assert ";;" in str(info.value)
    assert isinstance(info.value, ValueError)


def test_invalid_keys_rejected(view: Pair) -> None:
    with pytest.raises(ValueError):
        view.add("bad key", "x")
    with pytest.raises(ValueError):
        view.add("dimension", name="has space")
    with pytest.raises(TypeError):
        view.add("label", object())
