"""Tests for building LookML objects from Python and inserting them into documents."""

import pytest

from looker_cst import (
    Block,
    Case,
    Dimension,
    DimensionGroup,
    Explore,
    Join,
    Expr,
    Literal,
    Measure,
    Named,
    Quoted,
    View,
    parse,
)


def test_view_from_scratch() -> None:
    view = View(
        "users",
        Dimension("id", primary_key=True, type="number", sql="${TABLE}.id"),
        DimensionGroup("created", type="time", timeframes=["raw", "date", "week"], sql="${TABLE}.created_at"),
        Measure("count", type="count", drill_fields=["id"]),
        sql_table_name="public.users",
    )
    assert str(view) == (
        "view: users {\n"
        "  sql_table_name: public.users ;;\n"
        "\n"
        "  dimension: id {\n"
        "    primary_key: yes\n"
        "    type: number\n"
        "    sql: ${TABLE}.id ;;\n"
        "  }\n"
        "\n"
        "  dimension_group: created {\n"
        "    type: time\n"
        "    timeframes: [raw, date, week]\n"
        "    sql: ${TABLE}.created_at ;;\n"
        "  }\n"
        "\n"
        "  measure: count {\n"
        "    type: count\n"
        "    drill_fields: [id]\n"
        "  }\n"
        "}\n"
    )
    assert str(parse(str(view))) == str(view)


def test_explore_with_joins_and_keyword_fields() -> None:
    explore = Explore(
        "orders",
        Join("users", type="left_outer", relationship="many_to_one", sql_on="${orders.user_id} = ${users.id}"),
        Join("buyers", from_="users", sql_on="${orders.buyer_id} = ${buyers.id}"),
        label="Orders",
    )
    assert str(explore) == (
        "explore: orders {\n"
        '  label: "Orders"\n'
        "\n"
        "  join: users {\n"
        "    type: left_outer\n"
        "    relationship: many_to_one\n"
        "    sql_on: ${orders.user_id} = ${users.id} ;;\n"
        "  }\n"
        "\n"
        "  join: buyers {\n"
        "    from: users\n"
        "    sql_on: ${orders.buyer_id} = ${buyers.id} ;;\n"
        "  }\n"
        "}\n"
    )


def test_case_dimension() -> None:
    dimension = Dimension(
        "status",
        case=Case({"${TABLE}.status = 1": "Active", "${TABLE}.status = 0": "Inactive"}, else_="Unknown"),
    )
    assert str(dimension) == (
        "dimension: status {\n"
        "  case: {\n"
        "    when: {\n"
        "      sql: ${TABLE}.status = 1 ;;\n"
        '      label: "Active"\n'
        "    }\n"
        "    when: {\n"
        "      sql: ${TABLE}.status = 0 ;;\n"
        '      label: "Inactive"\n'
        "    }\n"
        '    else: "Unknown"\n'
        "  }\n"
        "}\n"
    )
    assert str(Dimension("s", case=Case([("a = 1", "A")]))) == (
        'dimension: s {\n  case: {\n    when: {\n      sql: a = 1 ;;\n      label: "A"\n    }\n  }\n}\n'
    )


def test_anonymous_blocks_and_repeated_keys() -> None:
    dimension = Dimension(
        "email",
        type="string",
        link=[Block(label="Profile", url="https://a.example/{{ value }}"), Block(label="Search", url="https://b")],
        filters={"status": "active"},
    )
    assert str(dimension) == (
        "dimension: email {\n"
        "  type: string\n"
        "  link: {\n"
        '    label: "Profile"\n'
        '    url: "https://a.example/{{ value }}"\n'
        "  }\n"
        "  link: {\n"
        '    label: "Search"\n'
        '    url: "https://b"\n'
        "  }\n"
        '  filters: [status: "active"]\n'
        "}\n"
    )
    derived = View("summary", derived_table=Block(sql="SELECT 1"))
    assert str(derived) == "view: summary {\n  derived_table: {\n    sql: SELECT 1 ;;\n  }\n}\n"


def test_named_covers_other_objects() -> None:
    parameter = Named("parameter", "metric", Named("allowed_value", None, label="Count", value="count"), type="unquoted")
    assert str(parameter) == (
        "parameter: metric {\n"
        "  type: unquoted\n"
        "  allowed_value: {\n"
        '    label: "Count"\n'
        '    value: "count"\n'
        "  }\n"
        "}\n"
    )


def test_add_to_existing_view_matches_siblings() -> None:
    source = (
        "view: users {\n"
        "  sql_table_name: public.users ;;\n"
        "  drill_fields: [id]\n"
        "\n"
        "  dimension: id {\n"
        "    type: number\n"
        "  }\n"
        "\n"
        "  measure: count {\n"
        "    type: count\n"
        "  }\n"
        "}\n"
    )
    doc = parse(source)
    view = doc.find("view", "users")
    added = Dimension("email", type="string", sql="${TABLE}.email").add_to(view, index=2)
    assert added == view.find("dimension", "email")
    assert str(doc) == source.replace(
        "  dimension: id {",
        "  dimension: email {\n    type: string\n    sql: ${TABLE}.email ;;\n  }\n\n  dimension: id {",
    )
    Measure("total", type="sum", sql="${TABLE}.amount").add_to(view)
    assert str(doc).endswith("  measure: total {\n    type: sum\n    sql: ${TABLE}.amount ;;\n  }\n}\n")


def test_add_to_document() -> None:
    doc = parse('connection: "prod"\n')
    Explore("users").add_to(doc)
    assert str(doc) == 'connection: "prod"\n\nexplore: users {}\n'


def test_forcing_a_values_form() -> None:
    measure = Measure(
        "count",
        type="count",
        value_format_name=Quoted("usd"),
        label=Literal("Count"),
        sql_distinct_key=Expr("${id}"),
    )
    assert str(measure) == (
        "measure: count {\n"
        "  type: count\n"
        '  value_format_name: "usd"\n'
        "  label: Count\n"
        "  sql_distinct_key: ${id} ;;\n"
        "}\n"
    )


def test_invalid_input() -> None:
    with pytest.raises(TypeError):
        View("users", "not a child")
    with pytest.raises(ValueError):
        Dimension("has space")
    with pytest.raises(TypeError):
        Named("view", "v", sql=Dimension("d"))
    with pytest.raises(TypeError):
        str(Dimension("d", label=object()))


def test_repr() -> None:
    assert repr(Dimension("id", type="number")) == "Dimension('id', type='number')"
    assert repr(Block(label="x")) == "Block(label='x')"
