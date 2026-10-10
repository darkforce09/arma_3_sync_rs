#!/usr/bin/env python3
"""SessionStart(compact|resume): re-inject orchestration state."""

from _common import REPO_ROOT


def head(path, lines):
    try:
        return "".join(path.open(encoding="utf-8").readlines()[:lines])
    except OSError:
        return ""


progress = head(REPO_ROOT / "docs" / "PROGRESS.md", 80)
loop = head(REPO_ROOT / "docs" / "ORCHESTRATION.md", 60)
if progress or loop:
    print("## Orchestration state (docs/PROGRESS.md, head)\n")
    print(progress)
    print("\n## Orchestration loop (docs/ORCHESTRATION.md, head)\n")
    print(loop)
