#!/usr/bin/env python3
"""PreToolUse(Bash): block destructive git, unbudgeted network access and raw cargo in worktrees."""

import re

from _common import block, in_worktree, is_subagent, read_input

WOG_HOST = "88.99.69.243"


def main() -> None:
    data = read_input()
    cmd = (data.get("tool_input") or {}).get("command") or ""
    cwd = data.get("cwd") or ""
    worktree = in_worktree(cwd)
    strict = worktree or is_subagent(data)

    if re.search(r"\bgit\s+push\b", cmd):
        block("guard_bash: git push is not allowed.")
    if re.search(r"\bgit\s+reset\s+--hard\b", cmd) and strict:
        block("guard_bash: git reset --hard is not allowed for agents.")
    if re.search(r"\bcargo\s+install\b", cmd):
        block("guard_bash: cargo install is not allowed; ask the user.")
    if re.search(r"\bcargo\s+update\b(?!.*\s-(p|w)\b)", cmd):
        block("guard_bash: bare cargo update would churn Cargo.lock; use -p <crate>.")
    if re.search(r"\b(curl|wget|lftp|ftp)\b", cmd) and WOG_HOST in cmd:
        block("guard_bash: live repository access only via scripts/gate.sh live (byte budget).")
    if strict:
        if re.search(r"\bgit\s+(checkout|switch)\s+(-\S+\s+)*main\b", cmd):
            block("guard_bash: agents never switch to main; the orchestrator merges.")
        if re.search(r"\bgit\s+worktree\s+remove\b", cmd):
            block("guard_bash: only the orchestrator removes worktrees.")
        if re.search(r"(^|[\s;&|(`])cargo\s", cmd) and "gate.sh" not in cmd:
            block(
                "guard_bash: run cargo through scripts/gate.sh "
                "(e.g. scripts/gate.sh cargo check -p <crate>) so the slot target dir is used."
            )


if __name__ == "__main__":
    main()
