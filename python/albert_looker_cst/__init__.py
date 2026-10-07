"""Lossless LookML parser.

Parse a LookML file into a tree of pairs, edit it, and write it back out with
every comment, blank line and indentation preserved.

    doc = parse(source)
    view = doc.find("view", "users")
    view.find("dimension", "id")["type"] = "string"
    assert str(parse(source)) == source
"""

from albert_looker_cst._native import Document, LookmlSyntaxError, Pair, parse_lookml

parse = parse_lookml

__all__ = ["Document", "LookmlSyntaxError", "Pair", "parse"]
