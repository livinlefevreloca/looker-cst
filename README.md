# looker-cst

A lossless LookML parser written in Rust with Python bindings. It parses a LookML file into a
concrete syntax tree (CST) that keeps every comment, blank line and indentation, so a file can be
read, modified and written back out with its formatting intact:

```python
from looker_cst import parse

doc = parse(source)
assert str(doc) == source  # byte for byte, for every file in the looker repo
```

The parser is built with [nom](https://github.com/rust-bakery/nom). The Python package is an
abi3 wheel built against Python 3.9, so one wheel works on Python 3.9 and 3.11.

## Python usage

```python
from looker_cst import parse, LookmlSyntaxError

doc = parse(path.read_text())

view = doc.find("view", "users")             # first `view: users { ... }`
dim = view.find("dimension", "id")

dim["type"]                                  # "number"
dim["type"] = "string"                       # keeps the value's kind: type: string
dim["label"] = "User ID"                     # adds a missing pair: label: "User ID"
dim["hidden"] = True                         # hidden: yes
dim["sql"] = "${TABLE}.user_id"              # sql: ${TABLE}.user_id ;;
dim.comments = ["# primary key"]             # rewrites the comment lines above the pair

email = view.add("dimension", name="email", index=2)   # new block, spaced like its siblings
email["type"] = "string"

measure = view.find("measure", "count")
measure["drill_fields"] = ["id", "email"]               # list layout and quoting are kept
measure["filters"] = {"status": "active"}               # filters: [status: "active"]

view.add_source("""
measure: total {
  type: sum
  sql: ${TABLE}.amount ;;
}
""")                                         # parsed and re-indented to fit

view.remove("dimension", name="legacy")      # removes the pair and its comments
dim.detach()                                 # removes a pair from wherever it is

path.write_text(str(doc))
```

### API

`parse(source) -> Document` raises `LookmlSyntaxError` (a `ValueError`) with `line`, `column`
and `offset` attributes.

`Document` and block `Pair`s are containers with the same methods:

| Method                                         | Description                                                     |
| ---------------------------------------------- | --------------------------------------------------------------- |
| `children`                                     | Direct child pairs                                              |
| `find(key, name=None)` / `find_all(...)`       | Child pairs by key, and block name                              |
| `get(key, default=None)`, `c[key]`             | Value of the first child with key                               |
| `c[key] = value`                               | Set the first child with key, adding it when missing            |
| `del c[key]`, `remove(pair_or_key, name=None)` | Remove a child and the comments above it                        |
| `add(key, value=None, *, name, kind, index)`   | Add a child; with no value it is a block, named when name given |
| `add_source(source, index=None)`               | Insert pairs parsed from a LookML snippet, re-indented          |
| `walk()`                                       | Every pair underneath, depth first                              |
| `len(c)`, `iter(c)`, `key in c`                | Child count, iteration, key membership                          |

`Pair` also has `key`, `name` (block name), `kind`, `value`, `set_value(value, kind=None)`,
`comments`, `parent`, `detach()` and `to_string(include_leading=False)`. Pairs are live handles:
editing a pair returned by `find` edits the document, and two handles to the same node compare
equal.

### Values

| `kind`    | LookML                     | Python value                         |
| --------- | -------------------------- | ------------------------------------ |
| `literal` | `type: number`             | `"number"`                           |
| `string`  | `label: "Say \"hi\""`      | `'Say "hi"'` (unescaped)             |
| `expr`    | `sql: ${TABLE}.id ;;`      | `"${TABLE}.id"`                      |
| `list`    | `filters: [a: "x", b, c]`  | `[("a", "x"), "b", "c"]`             |
| `block`   | `dimension: id { ... }`    | `None` (use the container methods)   |

Assigning to `value` (or `c[key]`) keeps the current kind, except that a literal becomes a
string when the new text needs quotes. A new value's kind is inferred: `sql`, `html`, `sql_*`
and `*_sql` keys are expressions; identifier-like text such as `number` is a literal, except
for keys that are conventionally quoted (`label`, `description`, `group_label`, ...); booleans
become `yes`/`no`; lists, tuples and dicts become lists. Pass `kind=` to choose explicitly.

New pairs copy the spacing of their neighbours, so a dimension added to a view gets the blank
line and indentation the other dimensions have, and a replaced list keeps its inline or
one-per-line layout, comma style and trailing comma.

### Building objects from Python

`View`, `Explore`, `Join`, `Dimension`, `DimensionGroup` and `Measure` build LookML from
Python values. Each fixes only its LookML key; fields are arbitrary keyword arguments, so any
parameter Looker supports can be written, and nested objects are passed positionally:

```python
from looker_cst import Block, Case, Dimension, Explore, Join, Measure, Named, Quoted, View

view = View(
    "users",
    Dimension("id", primary_key=True, type="number", sql="${TABLE}.id"),
    Dimension(
        "status",
        case=Case({"${TABLE}.status = 1": "Active", "${TABLE}.status = 0": "Inactive"}, else_="Unknown"),
    ),
    Dimension(
        "email",
        type="string",
        sql="${TABLE}.email",
        tags=["pii"],
        link=[Block(label="Profile", url="https://admin.example.com/{{ value }}")],
    ),
    Measure("count", type="count", drill_fields=["id", "email"], filters={"status": "Active"}),
    sql_table_name="public.users",
)
path.write_text(str(view))                       # a new file

doc = parse(path.read_text())
Dimension("name", type="string").add_to(doc.find("view", "users"), index=3)   # into a parsed file
Explore("users", Join("orders", from_="orders_v2", sql_on="${users.id} = ${orders.user_id}")).add_to(doc)
```

- Values convert as in `add`: `sql` style keys become `;;` expressions, booleans `yes`/`no`,
  lists `[a, b]` and dicts `[key: "value"]`.
- `Block(...)` is an unnamed `{ }` block such as `link` or `derived_table`; a list of them
  repeats the key. `Case(...)` writes a case's `when` blocks and `else`.
- `Quoted("usd")`, `Literal("Count")` and `Expr("${id}")` force a value's form.
- A trailing underscore is dropped from field names, for Python keywords: `from_`, `else_`.
- `Named(key, name, ...)` covers any other block, e.g. `Named("parameter", "metric", type="unquoted")`.
- `add_to` spaces the new object like its siblings; where there are none to copy, named blocks
  are set apart by a blank line and unnamed ones are not.

## Rust usage

```rust
let doc = looker_cst::parse(&source)?;
let view = doc.body.find("view", Some("users")).unwrap();
let id = view.read().body().unwrap().find("dimension", Some("id")).unwrap();
id.write().body_mut().unwrap().find("type", None).unwrap().write().set_text("string");
assert!(doc.to_string().contains("type: string"));
```

The tree is in `src/cst.rs`: every node owns the whitespace and comments before it (`leading`),
and a block or list owns the trivia before its closing bracket (`trailing`), so printing is a
plain concatenation. Editing helpers are in `src/edit.rs`.

## Development

```sh
cargo test                       # Rust unit, fixture and looker repo tests
uv run pytest                    # builds the extension, then runs the Python tests
cargo run --release --example roundtrip -- ../looker   # round trip check of a directory
uv run scripts/view_cst.py ../looker/models/analytics.model.lkml   # browse a file's tree
```

The looker repo tests read every `.lkml` file in the looker repo. For each file they check the
round trip, rewrite every value with itself, edit every value and read it back, and add then
remove pairs in every block, checking the source is restored exactly.

| Variable              | Default                                | Meaning                                               |
| --------------------- | -------------------------------------- | ----------------------------------------------------- |
| `LOOKER_REPO`         | `target/looker`                        | Checkout to test; cloned there when it does not exist |
| `LOOKER_REPO_URL`     | `https://github.com/mozilla/looker-hub.git` | Where to clone from                              |
| `REQUIRE_LOOKER_REPO` | unset                                  | Fail instead of skipping when the clone fails         |

The first run shallow clones `main` of [mozilla/looker-hub](https://github.com/mozilla/looker-hub)
into `target/looker` and later runs reuse it; delete the directory (or `cargo clean`) to pick up
newer commits. To test another checkout, point the tests at it:
`LOOKER_REPO=../looker-hub cargo test`.

### Benchmarks

```sh
cargo bench                                  # criterion: fixture, looker repo, largest file
uv run --with lkml scripts/benchmark.py      # Python: looker_cst against lkml
```

Both use the looker repo checkout the tests clone (`target/looker`, or `$LOOKER_REPO`), and
`scripts/benchmark.py` also takes a path. On an Apple Silicon laptop over the 3433 files
(71 MB) of [mozilla/looker-hub](https://github.com/mozilla/looker-hub), through Python (median
of 3 runs):

| Benchmark  | looker_cst | lkml       | lkml / ours |
| ---------- | ---------- | ---------- | ----------- |
| parse      | 266.6 ms   | 30823.7 ms | 116x        |
| print      | 59.9 ms    | 1551.5 ms  | 26x         |
| round trip | 304.2 ms   | 30796.7 ms | 101x        |

From Rust, `cargo bench` parses the same repo in about 240 ms (~280 MiB/s). The lkml comparison
takes several minutes on the full hub; pass a smaller directory to `scripts/benchmark.py` for a
quick check.

To test a specific Python version:

```sh
UV_PROJECT_ENVIRONMENT=.venv-3.9 uv run --python 3.9 pytest
```

To build a wheel:

```sh
uvx maturin build --release      # target/wheels/looker_cst-*-cp39-abi3-*.whl
```
