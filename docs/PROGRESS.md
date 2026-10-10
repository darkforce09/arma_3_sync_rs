# Rewrite progress

Mode: RUNNING            Updated: 2026-10-10
Current wave: 0          Next action: finish Wave 0 infra, then extract W0+W1 specs
Waiting-on: none
Blockers: WP01 needs JDK 25 (`sudo dnf install java-25-openjdk-devel`) — non-blocking for others

## In flight

| WP | Crate | State | Round | Agents | Slot | Branch | Worktree | HEAD | Since |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |

## Done before orchestration

| Item | Crate | Commit |
| --- | --- | --- |
| Workspace scaffold | all | d781711 |
| Java serialization codec | a3s_jserial | 7ca6421 |
| Domain types + Java codecs | a3s_types | c2bfc71 |
| `.a3s` gzip files | a3s_repo_files | 4230e76 |
| zsync + golden WOG fixtures | a3s_zsync | c016216 |
| SHA-1 cache | a3s_hash | 626cc7d |

## Waves

State values: TODO, SPEC, SPEC_OK, IMPL, VERIFY, FIX, REBASE, MERGED, BLOCKED, DEFERRED.

| Wave | WP | Crate(s) | Depends | State | Merge | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | infra | .claude, scripts, docs | — | IMPL | | |
| 0 | transport-commit | a3s_transport | — | IMPL | | download side, live WOG test |
| 0 | descriptor-lock | a3s_types tests | — | TODO | | |
| 0 | WP01 | java_fixture_harness | — | BLOCKED | | needs JDK 25 |
| 0 | WP02 | utils crates | — | TODO | | |
| 0 | WP03 | a3s_types local | — | TODO | | wire-touching |
| 0 | WP06 | a3s_transport upload | transport-commit | TODO | | |
| 0 | WP17 | a3s_tools bikey/rpt/radio | WP02 | TODO | | |
| 0 | WP20 | a3s_ui | — | TODO | | |
| 0 | WP31 | a3s_updater core | — | TODO | | |
| 1 | WP04 | a3s_repo_files local/legacy | WP02, WP03 | TODO | | |
| 1 | WP05 | autoconfig url/export | WP03 | TODO | | |
| 1 | WP07 | a3s_sync model/remote/status | WP02, WP03 | TODO | | critical path |
| 1 | WP11 | a3s_admin build | WP03 | TODO | | wire-touching |
| 1 | WP13 | a3s_addons | WP03 | TODO | | |
| 1 | WP15 | a3s_platform | WP02 | TODO | | |
| 2 | WP08 | a3s_sync plan | WP07 | TODO | | |
| 2 | WP09 | a3s_sync download | WP07 | TODO | | |
| 2 | WP12 | a3s_admin upload/check/events | WP06, WP07, WP11 | TODO | | |
| 2 | WP14 | app services | WP04, WP05 | TODO | | |
| 2 | WP16 | a3s_launch | WP13, WP15 | TODO | | |
| 2 | WP18 | presets | WP13, WP02 | TODO | | |
| 2 | WP32 | updater apply | WP31, WP15 | TODO | | |
| 3 | WP10 | sync pipeline | WP08, WP09 | TODO | | |
| 3 | WP21 | a3s_viewer_context | WP14, WP07 | TODO | | |
| 4 | WP19 | CLI | WP10, WP11, WP12, WP14, WP17 | TODO | | |
| 4 | WP22 | viewer shell | WP20, WP21, WP15, WP16 | TODO | | serial |
| 5 | WP23 | addons tab | WP22, WP13, WP18 | TODO | | |
| 5 | WP24 | options tabs | WP22, WP14, WP16 | TODO | | |
| 5 | WP25 | sync tab | WP22, WP21, WP07, WP14 | TODO | | |
| 5 | WP29 | dialogs | WP22, WP14, WP05, WP15 | TODO | | |
| 5 | WP30 | tool UIs | WP22, WP17, WP15 | TODO | | |
| 6 | WP26 | download tab | WP25, WP10 | TODO | | |
| 6 | WP27 | admin tab | WP25, WP11, WP12 | TODO | | |
| 6 | WP28 | events tab | WP25, WP12 | TODO | | |
| 7 | WP33 | packaging | WP19, WP22, WP32 | TODO | | |

## Decisions log

- D-1 2026-10-10: implementers in parallel worktrees, all agents Opus, continuous run, agents
  commit on branches, orchestrator merges (user).
- D-2 2026-10-10: `.pbo.zip` needs semantic compatibility only; `.a3s/*` and `.zsync` byte-identical (user).
- D-3 2026-10-10: cargo target dirs on Main_storage HDD (user).
- D-4 2026-10-10: local client data = new TOML format + one-time Java importer (user, earlier).
- D-5 2026-10-10: live tests use WOG FTP only; repo.ofcra.org is unreachable from this machine.
- D-6 2026-10-10: SFTP accepts any host key and logs its fingerprint (mirrors Java/MINA default).

## Follow-ups

## Live test log

| Date | Crate | Test | Result |
| --- | --- | --- | --- |

## Last reports
