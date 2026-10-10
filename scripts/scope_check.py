#!/usr/bin/env python3
"""Checks that a work-package branch only touches the paths its spec owns.

usage: scope_check.py --root <worktree> --spec <docs/specs/WPxx.md> [--base main]

The spec's "## Owned paths" section lists glob patterns in backticks, e.g.
`crates/net/a3s_sync/**`. Cargo.lock is always allowed.
"""

import argparse
import fnmatch
import pathlib
import re
import subprocess
import sys

ALWAYS_ALLOWED = ["Cargo.lock"]


def owned_patterns(spec: pathlib.Path) -> list[str]:
    text = spec.read_text(encoding="utf-8")
    match = re.search(r"^## Owned paths\s*$(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not match:
        sys.exit(f"{spec}: no '## Owned paths' section")
    patterns = []
    for line in match.group(1).splitlines():
        # Only the first backticked path of each bullet counts; prose mentions are ignored.
        bullet = re.match(r"\s*[-*]\s+`([^`]+)`", line)
        if bullet and "/" in bullet.group(1):
            patterns.append(bullet.group(1))
    return patterns


def matches(path: str, pattern: str) -> bool:
    if pattern.endswith("/**"):
        return path.startswith(pattern[:-3] + "/")
    return fnmatch.fnmatch(path, pattern)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--spec", required=True)
    parser.add_argument("--base", default="main")
    args = parser.parse_args()

    root = pathlib.Path(args.root)
    spec = pathlib.Path(args.spec)
    if not spec.is_absolute():
        # Specs live on main; prefer the main checkout's copy, fall back to the worktree.
        main_root = pathlib.Path(__file__).resolve().parent.parent
        spec = main_root / spec if (main_root / spec).exists() else root / spec
    patterns = owned_patterns(spec) + ALWAYS_ALLOWED

    diff = subprocess.run(
        ["git", "-C", str(root), "diff", "--name-only", f"{args.base}...HEAD"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()

    outside = [p for p in diff if not any(matches(p, pat) for pat in patterns)]
    if outside:
        print("SCOPE FAIL: files outside owned paths:")
        for p in outside:
            print(f"  {p}")
        print("owned:", ", ".join(patterns))
        return 1
    print(f"SCOPE PASS ({len(diff)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
