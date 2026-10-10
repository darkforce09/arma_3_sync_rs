"""Shared helpers for the project's Claude Code hooks."""

import json
import os
import pathlib
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKTREE_MARKER = "/.claude/worktrees/"


def read_input() -> dict:
    try:
        return json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        return {}


def block(reason: str) -> None:
    """Exit code 2 blocks the tool call and shows the reason to the model."""
    print(reason, file=sys.stderr)
    sys.exit(2)


def in_worktree(path: str) -> bool:
    return WORKTREE_MARKER in path.replace("\\", "/")


def is_subagent(data: dict) -> bool:
    return bool(data.get("agent_id") or data.get("agent_type"))


def repo_relative(path: str) -> str:
    """Path relative to the checkout (main repo or worktree) containing it."""
    p = path.replace("\\", "/")
    if WORKTREE_MARKER in p:
        rest = p.split(WORKTREE_MARKER, 1)[1]
        return rest.split("/", 1)[1] if "/" in rest else ""
    try:
        return pathlib.Path(os.path.abspath(p)).relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return p
