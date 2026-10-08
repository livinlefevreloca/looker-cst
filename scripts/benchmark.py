"""Compare looker_cst with lkml on every .lkml file in a repo.

Times parsing, printing a parsed tree, and the full round trip, reporting the median of
several runs over the whole repo. Defaults to the checkout the tests clone into, target/looker.

    uv run --with lkml scripts/benchmark.py
    uv run --with lkml scripts/benchmark.py /tmp/looker-hub --repeat 3
"""

import argparse
import statistics
import sys
import time
from pathlib import Path
from typing import Callable, Dict, List

import looker_cst

try:
    import lkml
except ImportError:
    lkml = None

DEFAULT_REPO = Path(__file__).resolve().parents[1] / "target" / "looker"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", type=Path, nargs="?", default=DEFAULT_REPO, help="directory of .lkml files")
    parser.add_argument("--repeat", type=int, default=5, help="runs per benchmark; the median is reported")
    args = parser.parse_args()

    if not args.repo.is_dir():
        print(f"{args.repo} not found; run the tests once to clone it, or pass a path", file=sys.stderr)
        return 2
    sources = [p.read_text() for p in sorted(args.repo.rglob("*.lkml")) if ".git" not in p.parts]
    megabytes = sum(len(s.encode()) for s in sources) / 1e6
    print(f"{len(sources)} files, {megabytes:.1f} MB, median of {args.repeat} runs\n")

    results: Dict[str, Dict[str, float]] = {"looker_cst": benchmark_looker_cst(sources, args.repeat)}
    if lkml is None:
        print("lkml is not installed; run with `uv run --with lkml` to compare\n", file=sys.stderr)
    else:
        results["lkml"] = benchmark_lkml(sources, args.repeat)
    print_table(results, megabytes)
    return 0


def benchmark_looker_cst(sources: List[str], repeat: int) -> Dict[str, float]:
    """Times looker_cst over all sources."""
    docs = [looker_cst.parse(s) for s in sources]
    return {
        "parse": median_seconds(lambda: [looker_cst.parse(s) for s in sources], repeat),
        "print": median_seconds(lambda: [str(d) for d in docs], repeat),
        "round trip": median_seconds(lambda: [str(looker_cst.parse(s)) for s in sources], repeat),
    }


def benchmark_lkml(sources: List[str], repeat: int) -> Dict[str, float]:
    """Times lkml's lossless parse tree over all sources, plus its dict based load for reference."""
    trees = [lkml.parse(s) for s in sources]
    return {
        "parse": median_seconds(lambda: [lkml.parse(s) for s in sources], repeat),
        "print": median_seconds(lambda: [str(t) for t in trees], repeat),
        "round trip": median_seconds(lambda: [str(lkml.parse(s)) for s in sources], repeat),
        "load to dict": median_seconds(lambda: [lkml.load(s) for s in sources], repeat),
    }


def median_seconds(run: Callable[[], object], repeat: int) -> float:
    """Median wall time of run over repeat calls."""
    times = []
    for _ in range(repeat):
        started = time.perf_counter()
        run()
        times.append(time.perf_counter() - started)
    return statistics.median(times)


def print_table(results: Dict[str, Dict[str, float]], megabytes: float) -> None:
    """Prints each benchmark's time per library, and lkml's time as a multiple of ours."""
    ours = results["looker_cst"]
    theirs = results.get("lkml", {})
    print(f"{'benchmark':<14}{'looker_cst':>20}{'lkml':>12}{'lkml / ours':>14}")
    for name in dict.fromkeys([*ours, *theirs]):
        mine = f"{ours[name] * 1000:.1f} ms" if name in ours else "-"
        other = f"{theirs[name] * 1000:.1f} ms" if name in theirs else "-"
        ratio = f"{theirs[name] / ours[name]:.0f}x" if name in ours and name in theirs else "-"
        print(f"{name:<14}{mine:>20}{other:>12}{ratio:>14}")
    print(f"\nlooker_cst parses at {megabytes / ours['parse']:.0f} MB/s")


if __name__ == "__main__":
    sys.exit(main())
