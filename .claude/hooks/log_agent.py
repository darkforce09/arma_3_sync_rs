#!/usr/bin/env python3
"""SubagentStop: append a line to .claude/logs/agents.log."""

import datetime
import json

from _common import REPO_ROOT, read_input

data = read_input()
log_dir = REPO_ROOT / ".claude" / "logs"
log_dir.mkdir(parents=True, exist_ok=True)
entry = {
    "time": datetime.datetime.now().isoformat(timespec="seconds"),
    "agent_id": data.get("agent_id"),
    "agent_type": data.get("agent_type"),
    "cwd": data.get("cwd"),
    "session_id": data.get("session_id"),
}
with (log_dir / "agents.log").open("a", encoding="utf-8") as f:
    f.write(json.dumps(entry) + "\n")
