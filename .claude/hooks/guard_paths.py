#!/usr/bin/env python3
"""PreToolUse(Edit|Write|MultiEdit|NotebookEdit): protect reference code, fixtures and config."""

import fnmatch

from _common import block, in_worktree, is_subagent, read_input, repo_relative

ALWAYS = ["java_version/*"]

SUBAGENT = [
    "tests/fixtures/*",
    ".claude/agents/*",
    ".claude/hooks/*",
    ".claude/settings*.json",
    ".claude/commands/*",
    ".github/*",
    "deny.toml",
    "rust-toolchain.toml",
    "clippy.toml",
    "Cargo.lock",
    "crates/store/a3s_types/tests/descriptor_lock.rs",
    "scripts/*",
]

WORKTREE = ["Cargo.toml", "docs/PROGRESS.md", "docs/ORCHESTRATION.md", "docs/specs/*"]


def main() -> None:
    data = read_input()
    tool_input = data.get("tool_input") or {}
    path = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
    if not path:
        return
    rel = repo_relative(path)
    cwd = data.get("cwd") or ""
    worktree = in_worktree(path) or in_worktree(cwd)
    strict = is_subagent(data) or worktree

    rules = list(ALWAYS)
    if strict:
        rules += SUBAGENT
    if worktree:
        rules += WORKTREE
    for pattern in rules:
        if rel == pattern or fnmatch.fnmatch(rel, pattern):
            block(
                f"guard_paths: '{rel}' is protected ({pattern}). "
                "If you need a change there, stop and report it to the orchestrator "
                "(NEEDS_WORKSPACE_DEP / NEEDS_UPSTREAM_CHANGE / NEEDS_DECISION)."
            )


if __name__ == "__main__":
    main()
