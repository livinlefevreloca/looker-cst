"""Lossless LookML parser.

Parse a LookML file into a tree of pairs, edit it, and write it back out with
every comment, blank line and indentation preserved.

    doc = parse(source)
    view = doc.find("view", "users")
    view.find("dimension", "id")["type"] = "string"
    assert str(parse(source)) == source
"""

from looker_cst._native import Document, LookmlSyntaxError, Pair, parse_lookml
from looker_cst.builders import (
    Block,
    Case,
    Dimension,
    DimensionGroup,
    Explore,
    Expr,
    Join,
    Literal,
    Measure,
    Named,
    Quoted,
    View,
)

parse = parse_lookml

__all__ = [
    "Block",
    "Case",
    "Dimension",
    "DimensionGroup",
    "Document",
    "Explore",
    "Expr",
    "Join",
    "Literal",
    "LookmlSyntaxError",
    "Measure",
    "Named",
    "Pair",
    "Quoted",
    "View",
    "parse",
]
