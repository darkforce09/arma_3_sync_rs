# Orchestration of the Rust rewrite

How the main Claude session ("orchestrator") drives sub-agents through the remaining work
packages (WPs). State lives in [`PROGRESS.md`](PROGRESS.md); specs in [`specs/`](specs/).

## Invariants

1. `main` is always green: `scripts/gate.sh --slot main workspace` passes (golden + descriptor
   lock included).
2. `main` is committed before spawning an implementer — worktrees branch from `HEAD`.
3. Only the orchestrator merges, edits the root `Cargo.toml`, writes `docs/PROGRESS.md` and
   commits specs. On `main`, stage explicit paths (never `git add -A`): spec extractors may be
   writing files concurrently.
4. At most **3 compiling agents** (implementers / verifiers holding slots 1–3). Builds share an
   HDD; drop to 2 if I/O thrashes. Spec extractors and reviewers are not limited.
5. All cargo runs go through `scripts/gate.sh` (per-slot target dirs under
   `/run/media/system/Main_storage/cargo-target/a3s/`).

## Agents

| Agent | Spawn | Purpose |
| --- | --- | --- |
| `java-spec-extractor` | background, no isolation | writes `docs/specs/<WP>.md` |
| `rust-implementer` | background, `isolation: "worktree"` | implements a spec, commits on its branch |
| `egui-implementer` | background, `isolation: "worktree"` | same for viewer crates |
| `compat-verifier` | background, no isolation, given `WT` path + slot | gates, live tests, adversarial tests |
| `reviewer` | background, no isolation, given `WT` path | read-only review vs spec + Java |
| `merge-fixer` | `isolation: "worktree"` | resolves cross-branch conflicts / red main |

Custom agent types are registered when a session starts. In a session where they are not
available yet, spawn `general-purpose` with `model: "opus"` and start the prompt with
`ROLE: First read <repo>/.claude/agents/<name>.md and follow it exactly as your instructions.`

Prompts to implementers contain: WP id, spec path, slot number, merged upstream WPs and their
APIs. Prompts to verifier/reviewer contain: WP id, `WT` absolute path, slot, spec path,
wire-touching flag, allowed live tests.

## Per-WP pipeline

```
SPEC → (decisions) → SPEC_OK → IMPL (worktree, slot k) → VERIFY ∥ REVIEW
                                    ↑                         │ fail (round ≤ 3)
                                    └──── FIX (SendMessage) ◄──┤
                                                               │ pass
                         REBASE onto main (SendMessage) ◄──────┘
                                    │
                        MERGE (orchestrator, slot main) → CLEANUP → MERGED
```

1. **SPEC.** Spawn `java-spec-extractor`. Read its report. Resolve safe open questions (Java
   behaviour wins); escalate wire-format or deviation questions to the user (blocks only that
   WP). Commit `docs(specs): <WP>`.
2. **Wave pre-step** (once per wave): add every spec's "Workspace deps needed" to the root
   `[workspace.dependencies]`, run `cargo deny check`, commit `chore(deps): ...`. Viewer waves
   also commit the panel/registration scaffold first.
3. **IMPL.** Spawn the implementer. Record agent id, slot, branch, start time in PROGRESS.
   - `NEEDS_WORKSPACE_DEP`: add it on main, commit, SendMessage "rebase on main and continue".
   - `NEEDS_UPSTREAM_CHANGE`: if the upstream crate is merged and the change is additive,
     amend the spec's owned paths (commit on main) and tell the implementer to rebase; else
     queue a micro-WP after the upstream merges.
   - `NEEDS_DECISION`: decide if not wire-related, else ask the user.
4. **VERIFY ∥ REVIEW.** Spawn both. On re-review after a fix round, pass the previous HEAD so
   only the delta is reviewed.
5. **FIX.** Verifier FAIL (LOGIC/COMPAT) or reviewer BLOCKER/MAJOR → SendMessage the
   implementer the condensed findings (≤ 25 lines). Max 3 rounds, then the WP is BLOCKED.
   FAIL-NETWORK alone never fails a WP; log it and re-run live later.
6. **REBASE** if `main` moved: SendMessage "git rebase main, resolve, gate.sh crate, report".
7. **MERGE** (main checkout):
   ```
   python3 scripts/scope_check.py --root <WT> --spec docs/specs/<WP>.md
   git merge --no-ff --no-commit <branch>
   # Cargo.lock conflict only: git checkout --ours Cargo.lock
   #   && scripts/gate.sh --slot main cargo check --workspace && git add Cargo.lock
   # other conflicts: git merge --abort → rebase request or merge-fixer
   scripts/gate.sh --slot main workspace
   # PASS → git commit (subject "Merge <WP>: <title>", trailers Work-Package + Co-Authored-By)
   # FAIL → git merge --abort → implementer fix round or merge-fixer
   ```
8. **CLEANUP.** `git worktree remove <WT>`, `git branch -d <branch>`, free the slot, update and
   commit PROGRESS (`chore(progress): <WP> merged`).

## Per-wave loop

```
for wave in waves:
  specs of the wave are SPEC_OK (extracted during the previous wave)
  wave pre-step (deps, scaffolds) → commit
  while runnable WPs or WPs in flight:
     fill free slots (≤ 3) with WPs whose dependencies are MERGED
     on each agent notification: advance that WP (pipeline above)
     merge one WP at a time, in dependency order
     extract specs for the next wave when idle
  wave exit: gate.sh --slot main workspace; verifier live run for net crates; df check
  record wave summary in PROGRESS; mark a chapter; continue
```

## Stop and ask the user

- A WP is BLOCKED after 3 rounds and nothing else is runnable.
- A decision touches the wire format or needs a deviation from Java behaviour.
- `main` is red and one merge-fixer round did not fix it.
- A golden or descriptor-lock test fails on `main`.
- Free space on Main_storage drops below 20 GB.
- A required dependency fails `cargo deny`.
- The next runnable step needs the JDK and none is installed.

Network flakiness never stops the run.

## Recovery

- After context compaction: the SessionStart hook re-injects PROGRESS and this loop; agent ids
  in PROGRESS remain valid for SendMessage within the same session.
- After a session restart: agent ids are gone, worktrees and branches remain. For each
  in-flight WP check `git worktree list` and `git log main..<branch>`, spawn a fresh implementer
  told to `git merge --ff-only <old-branch>` in its new worktree and continue from the state
  recorded in PROGRESS, then remove the old worktree.
