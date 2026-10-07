"""Reads, modifies and rewrites every .lkml file in the looker repo through the Python API.

The repo is found at LOOKER_REPO, or ../looker next to this one. Without it these tests are
skipped, unless REQUIRE_LOOKER_REPO is set.
"""

import os
from pathlib import Path
from typing import List

import pytest

from albert_looker_cst import Pair, parse

LOOKER_REPO = Path(os.environ.get("LOOKER_REPO", Path(__file__).parents[2].parent / "looker"))
FILES = sorted(p for p in LOOKER_REPO.rglob("*.lkml") if ".git" not in p.parts) if LOOKER_REPO.is_dir() else []

if not FILES and os.environ.get("REQUIRE_LOOKER_REPO"):
    raise RuntimeError(f"no .lkml files found under {LOOKER_REPO}")

pytestmark = pytest.mark.skipif(not FILES, reason=f"looker repo not found at {LOOKER_REPO}")


def ids(path: Path) -> str:
    return str(path.relative_to(LOOKER_REPO))


def outline(pairs: List[Pair]) -> List[str]:
    return [f"{p.key}:{p.name}:{p.kind}:{p.value!r}" for p in pairs]


@pytest.mark.parametrize("path", FILES, ids=ids)
def test_read_modify_write(path: Path) -> None:
    source = path.read_text()
    doc = parse(source)
    assert str(doc) == source

    before = outline(doc.walk())
    blocks = [p for p in doc.walk() if p.kind == "block"]
    added = []
    for block in blocks:
        added.append(block.add("lkml_cst_marker", "first", index=0))
        added.append(block.add("lkml_cst_marker", name="last"))
    for pair in doc.walk():
        if pair.kind == "string":
            pair.value = pair.value + ' "edited"'

    edited = str(doc)
    reparsed = parse(edited)
    assert str(reparsed) == edited
    assert sum(p.key == "lkml_cst_marker" for p in reparsed.walk()) == len(added)

    for pair in reparsed.walk():
        if pair.kind == "string":
            assert pair.value.endswith(' "edited"')
            pair.value = pair.value[: -len(' "edited"')]
    for pair in reparsed.walk():
        if pair.key == "lkml_cst_marker":
            assert pair.detach()
    assert outline(reparsed.walk()) == before
    assert str(reparsed) == source
