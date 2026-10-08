"""Parse every .lkml file in a repo and write it back out, to check the round trip is lossless.

Rewrites the files in place by default, so `git -C <repo> diff` shows anything the parser
changed. With --out the repo is copied to a separate directory and rewritten there instead.

    uv run --with lkml scripts/rewrite_repo.py ../looker --lib lkml
    uv run scripts/rewrite_repo.py ../looker --lib looker_cst
    git -C ../looker status --short    # empty when the round trip is lossless

With --insert and --view, the LookML in a file (such as a dimension) is also added to the end
of that view, so the diff shows only the inserted lines:

    uv run scripts/rewrite_repo.py ../looker --lib looker_cst \\
        --insert new_dimension.lkml --view users
    git -C ../looker diff
"""

import argparse
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Callable, Dict, List, Optional


def lkml_round_trip(source: str) -> str:
    """Round trips through lkml's lossless parse tree."""
    import lkml

    return str(lkml.parse(source))


def looker_cst_round_trip(source: str) -> str:
    """Round trips through looker_cst."""
    import looker_cst

    return str(looker_cst.parse(source))


LIBS: Dict[str, Callable[[str], str]] = {
    "lkml": lkml_round_trip,
    "looker_cst": looker_cst_round_trip,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", type=Path, help="directory to read .lkml files from")
    parser.add_argument("--lib", choices=sorted(LIBS), required=True, help="parser to round trip with")
    parser.add_argument("--out", type=Path, help="write to this directory instead of in place")
    parser.add_argument("--force", action="store_true", help="rewrite in place even with uncommitted changes")
    parser.add_argument("--insert", type=Path, help="file of LookML to add to the end of --view")
    parser.add_argument("--view", help="name of the view to add --insert to, e.g. users or +users")
    args = parser.parse_args()

    if (args.insert is None) != (args.view is None):
        parser.error("--insert and --view must be given together")
    if args.insert and args.lib != "looker_cst":
        parser.error("--insert is only supported with --lib looker_cst")

    repo: Path = args.repo.resolve()
    out: Optional[Path] = args.out.resolve() if args.out else None
    if out is None and not args.force and has_uncommitted_changes(repo):
        print(f"{repo} has uncommitted changes; commit or stash them, or pass --force or --out", file=sys.stderr)
        return 2

    files = sorted(p for p in repo.rglob("*.lkml") if ".git" not in p.relative_to(repo).parts)
    insert_into: Optional[Path] = None
    if args.insert:
        matches = files_defining_view(files, args.view)
        if len(matches) != 1:
            found = ", ".join(str(p.relative_to(repo)) for p in matches) or "no files"
            print(f"view {args.view!r} must be defined in exactly one file, found {found}", file=sys.stderr)
            return 2
        insert_into = matches[0]

    if out:
        # Copy everything first so a recursive diff only reports files the parser changed.
        shutil.copytree(repo, out, ignore=shutil.ignore_patterns(".git"), dirs_exist_ok=True)

    round_trip = LIBS[args.lib]
    changed, failed = 0, 0
    started = time.perf_counter()
    for path in files:
        # Bytes in and out, so no newline translation can hide or cause a difference.
        source = path.read_bytes().decode("utf-8")
        try:
            if path == insert_into:
                rewritten = insert_into_view(source, args.view, args.insert.read_text())
            else:
                rewritten = round_trip(source)
        except Exception as error:
            failed += 1
            print(f"FAILED {path.relative_to(repo)}: {error}", file=sys.stderr)
            continue
        if rewritten != source:
            changed += 1
            print(f"CHANGED {path.relative_to(repo)}")
        target = out / path.relative_to(repo) if out else path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(rewritten.encode("utf-8"))
    elapsed = time.perf_counter() - started

    print(f"{args.lib}: {len(files)} files rewritten in {elapsed:.2f}s, {changed} changed, {failed} failed to parse")
    if insert_into:
        print(f"inserted {args.insert} at the end of view {args.view!r} in {insert_into.relative_to(repo)}")
    if out:
        print(f"compare with: diff -r --exclude=.git {repo} {out}")
    else:
        print(f"check with: git -C {repo} diff")
    expected_changes = 1 if insert_into else 0
    return 1 if changed != expected_changes or failed else 0


def files_defining_view(files: List[Path], view: str) -> List[Path]:
    """Files with a top level view block of this name."""
    import looker_cst

    matches = []
    for path in files:
        source = path.read_text()
        if view in source and looker_cst.parse(source).find("view", view) is not None:
            matches.append(path)
    return matches


def insert_into_view(source: str, view: str, snippet: str) -> str:
    """Adds the pairs in snippet to the end of the view, checking the result parses back."""
    import looker_cst

    doc = looker_cst.parse(source)
    added = doc.find("view", view).add_source(snippet)
    rewritten = str(doc)
    reparsed = looker_cst.parse(rewritten).find("view", view)
    expected = [(p.key, p.name) for p in added]
    if [(p.key, p.name) for p in reparsed.children[-len(added) :]] != expected:
        raise RuntimeError("inserted pairs were not found at the end of the view after reparsing")
    return rewritten


def has_uncommitted_changes(repo: Path) -> bool:
    """True when the repo's working tree differs from its last commit."""
    result = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain"], capture_output=True, text=True, check=False
    )
    return result.returncode == 0 and bool(result.stdout.strip())


if __name__ == "__main__":
    sys.exit(main())
