"""Reads, modifies and rewrites every .lkml file in the looker repo through the Python API.

The repo is read from LOOKER_REPO, which defaults to target/looker. When that path does not
exist the repo is cloned there from LOOKER_REPO_URL (default mozilla/looker-hub over HTTPS), so
point LOOKER_REPO at an existing checkout to test a branch. If the clone
fails these tests are skipped, unless REQUIRE_LOOKER_REPO is set.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import List, Optional

import pytest

from looker_cst import Pair, parse

PROJECT_ROOT = Path(__file__).parents[2]
LOOKER_REPO = Path(os.environ.get("LOOKER_REPO", PROJECT_ROOT / "target" / "looker"))
LOOKER_REPO_URL = os.environ.get("LOOKER_REPO_URL", "https://github.com/mozilla/looker-hub.git")


def clone_looker_repo() -> Optional[str]:
    """Shallow clones the looker repo to LOOKER_REPO, returning why it failed if it did.

    The clone goes to a temporary directory first so an interrupted clone is never mistaken
    for a complete checkout.
    """
    partial = LOOKER_REPO.with_name(f"{LOOKER_REPO.name}.partial-{os.getpid()}")
    LOOKER_REPO.parent.mkdir(parents=True, exist_ok=True)
    print(f"cloning {LOOKER_REPO_URL} to {LOOKER_REPO}", file=sys.stderr)
    result = subprocess.run(
        ["git", "clone", "--quiet", "--depth", "1", LOOKER_REPO_URL, str(partial)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        shutil.rmtree(partial, ignore_errors=True)
        return result.stderr.strip()
    try:
        partial.rename(LOOKER_REPO)
    except OSError:
        # Another test process finished its clone first; use that one.
        shutil.rmtree(partial, ignore_errors=True)
    return None


CLONE_ERROR = None if LOOKER_REPO.is_dir() else clone_looker_repo()
if CLONE_ERROR and os.environ.get("REQUIRE_LOOKER_REPO"):
    raise RuntimeError(f"could not clone {LOOKER_REPO_URL} to {LOOKER_REPO}: {CLONE_ERROR}")

FILES = sorted(p for p in LOOKER_REPO.rglob("*.lkml") if ".git" not in p.parts) if LOOKER_REPO.is_dir() else []
if LOOKER_REPO.is_dir() and not FILES:
    raise RuntimeError(f"no .lkml files found under {LOOKER_REPO}")

pytestmark = pytest.mark.skipif(
    not FILES, reason=f"could not clone {LOOKER_REPO_URL} to {LOOKER_REPO}: {CLONE_ERROR}"
)


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
