# File header comment
include: "/views/base.view"   # trailing comment after a string

view: +users {   # refinement with a comment after the brace
  extends: [base_users]
  sql_table_name:
    analytics.users ;;

	# tab indented comment
  dimension: id {
    primary_key: yes
    type: number
    sql: ${TABLE}.id;;
  }

  dimension: quoted {
    label: "Has \"quotes\" and a \\ backslash"
    description: "Spans
multiple lines"
    sql:
      CASE
        WHEN ${TABLE}.status = 'a;b' THEN 'x'
        ELSE 'y'
      END
    ;;
    html: {% if value == 'x' %}<b>{{ value }}</b>{% else %}{{ value }}{% endif %} ;;
  }

  dimension: status {
    case: {
      when: {
        sql: ${TABLE}.status = 1 ;;
        label: "Active"
      }
      else: "Inactive"
    }
  }

  dimension_group: created {
    type: time
    timeframes: [
      raw,
      date, # inline comment inside a list
      week,
      # comment on its own line in a list
      month,
    ]
    sql: ${TABLE}.created_at ;;
  }

  measure: total {
    type: sum
    sql: ${TABLE}.amount ;;
    filters: [status: "active", created_date: "last 30 days"]
    value_format: "$#,##0.00"
    drill_fields: [detail*, -users.id]
    link: {
      label: "Open"
      url: "https://example.com/{{ value }}"
    }
  }

  parameter: choice {
    allowed_value: { label: "A" value: "a" }
    allowed_value: {label:"B" value:"b"}
  }

  filter : spaced_colon {
    type : string
  }

  set: empty {
    fields: []
  }

  set: detail {
    fields: [id, created_date]
  }
}

explore: users {}
explore: orders{
  sql_always_where: ${orders.deleted} = 0 ;;
  join: users {
    type: left_outer
    relationship: many_to_one
    sql_on: ${orders.user_id} = ${users.id} ;;
  }
}
datagroup: nightly {
  sql_trigger_value: SELECT CURRENT_DATE ;;
  max_cache_age: "24 hours"
}
