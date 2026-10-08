"""Build LookML objects from Python and insert them into a parsed document.

Each type fixes only its LookML key. Fields are arbitrary keyword arguments, so any parameter
Looker supports can be written without this module knowing about it, and nested objects such
as the dimensions of a view are passed positionally:

    view = View(
        "users",
        Dimension("id", primary_key=True, type="number", sql="${TABLE}.id"),
        Measure("count", type="count"),
        sql_table_name="public.users",
    )
    path.write_text(str(view))           # a new file
    view.add_to(doc)                     # or into a parsed document, spaced like its siblings

Field values are converted as Document.add converts them: sql style keys become `;;`
expressions, booleans become yes/no, lists become lists and dicts become `[key: "value"]`
lists. Block(...) writes an unnamed `{ }` block, a list of them repeats the key, Case(...)
writes a case, and Quoted, Literal and Expr force a value's form. A trailing underscore is
dropped from field names so Python keywords can be used, as in Join("buyers", from_="users").
"""

from typing import Any, ClassVar, Iterable, Mapping, Optional, Tuple, Union

from looker_cst._native import Document, Pair, parse_lookml

Container = Union[Document, Pair]

# Characters that end a LookML key or name, so they cannot appear in a block name.
_NAME_STOP_CHARS = set(',[]{}":#;')


class _Forced:
    """A field value written in a fixed form instead of the inferred one."""

    kind: ClassVar[str]

    def __init__(self, text: str) -> None:
        self.text = text

    def __repr__(self) -> str:
        return f"{type(self).__name__}({self.text!r})"


class Quoted(_Forced):
    """A value written as a double quoted string, as in value_format_name: "usd"."""

    kind = "string"


class Literal(_Forced):
    """A value written unquoted, as in type: number."""

    kind = "literal"


class Expr(_Forced):
    """A value written as a raw expression ending in ;;, for keys not named like sql keys."""

    kind = "expr"


class Case:
    """The value of a case dimension: a when block per condition, then an optional else label.

    whens maps each SQL condition to its label, in order, or is a sequence of (sql, label) pairs.
    """

    def __init__(self, whens: Union[Mapping[str, str], Iterable[Tuple[str, str]]], else_: Optional[str] = None):
        self.whens = list(whens.items()) if isinstance(whens, Mapping) else list(whens)
        self.else_ = else_

    def _fill(self, case: Pair) -> None:
        """Adds the when blocks and else label to an empty case block."""
        for sql, label in self.whens:
            when = case.add("when")
            when.add("sql", sql)
            when.add("label", label, kind="string")
        if self.else_ is not None:
            case.add("else", self.else_, kind="string")

    def __repr__(self) -> str:
        return f"Case({dict(self.whens)!r}, else_={self.else_!r})"


class Block:
    """An unnamed block, used as a field value as in link=Block(label="Docs", url="...")."""

    def __init__(self, *children: "Named", **fields: Any) -> None:
        for child in children:
            if not isinstance(child, Named):
                raise TypeError(f"positional arguments must be LookML objects such as Dimension, not {child!r}")
        for key, value in fields.items():
            if isinstance(value, Named) or (isinstance(value, list) and any(isinstance(v, Named) for v in value)):
                raise TypeError(f"{key}= cannot hold a {type(value).__name__}; pass it positionally instead")
        self.children = children
        self.fields = fields

    def _fill(self, block: Pair) -> None:
        """Adds this object's fields, then its children, to an empty block."""
        for key, value in self.fields.items():
            _add_field(block, key[:-1] if key.endswith("_") else key, value)
        for child in self.children:
            child.add_to(block)

    def _arguments(self) -> str:
        parts = [repr(child) for child in self.children]
        parts.extend(f"{key}={value!r}" for key, value in self.fields.items())
        return ", ".join(parts)

    def __repr__(self) -> str:
        return f"Block({self._arguments()})"


class Named(Block):
    """Any LookML block, named as in parameter: metric { ... } or unnamed when name is None."""

    def __init__(self, key: str, name: Optional[str], *children: "Named", **fields: Any) -> None:
        if name is not None and (not name or any(c.isspace() or c in _NAME_STOP_CHARS for c in name)):
            raise ValueError(f"{name!r} is not a valid LookML name")
        super().__init__(*children, **fields)
        self.key = key
        self.name = name

    def add_to(self, container: Container, index: Optional[int] = None) -> Pair:
        """Inserts this object into a document or block, at the end unless index is given."""
        pair = container.add(self.key, name=self.name, index=index)
        self._fill(pair)
        return pair

    def __str__(self) -> str:
        doc = parse_lookml("")
        self.add_to(doc)
        return str(doc)

    def __repr__(self) -> str:
        arguments = self._arguments()
        return f"Named({self.key!r}, {self.name!r}{', ' if arguments else ''}{arguments})"


class _Typed(Named):
    """A Named block whose key is fixed by the class."""

    KEY: ClassVar[str]

    def __init__(self, name: str, *children: Named, **fields: Any) -> None:
        super().__init__(self.KEY, name, *children, **fields)

    def __repr__(self) -> str:
        arguments = self._arguments()
        return f"{type(self).__name__}({self.name!r}{', ' if arguments else ''}{arguments})"


class View(_Typed):
    """A view: name { ... } block."""

    KEY = "view"


class Explore(_Typed):
    """An explore: name { ... } block."""

    KEY = "explore"


class Join(_Typed):
    """A join: name { ... } block inside an explore."""

    KEY = "join"


class Dimension(_Typed):
    """A dimension: name { ... } block inside a view."""

    KEY = "dimension"


class DimensionGroup(_Typed):
    """A dimension_group: name { ... } block inside a view."""

    KEY = "dimension_group"


class Measure(_Typed):
    """A measure: name { ... } block inside a view."""

    KEY = "measure"


def _add_field(container: Pair, key: str, value: Any) -> None:
    """Adds one key: value field to a block, expanding blocks, cases and forced values."""
    if isinstance(value, Block):
        value._fill(container.add(key))
    elif isinstance(value, Case):
        value._fill(container.add(key))
    elif isinstance(value, _Forced):
        container.add(key, value.text, kind=value.kind)
    elif isinstance(value, list) and value and all(isinstance(item, Block) for item in value):
        for item in value:
            item._fill(container.add(key))
    else:
        container.add(key, value)
