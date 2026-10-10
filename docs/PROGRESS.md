# Rewrite progress

Mode: PAUSING (user: pause after current agents finish; spawn nothing new)            Updated: 2026-10-10
Current wave: 0          Next action: wait for W0 implementers (WP03, WP02, WP06) and W1 spec extractors; then verify/review/merge; start WP20, WP17, WP31 as slots free
Waiting-on: impl WP03 (slot 1), impl WP02 (slot 2), impl WP06 (slot 3); specs WP04, WP05, WP07, WP11, WP13, WP15
Blockers: WP01 needs JDK 25 (`sudo dnf install java-25-openjdk-devel`) — non-blocking for others

## In flight

| WP | Crate | State | Round | Agents | Slot | Branch | Worktree | HEAD | Since |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| WP03 | a3s_types local | IMPL | 0 | impl=a2f164b1 | 1 | (worktree) | | | 2026-10-10 |
| WP02 | utils | IMPL | 0 | impl=a453ddf6 | 2 | (worktree) | | | 2026-10-10 |
| WP06 | a3s_transport upload | IMPL | 0 | impl=a705b69e | 3 | (worktree) | | | 2026-10-10 |

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
| 0 | infra | .claude, scripts, docs | — | MERGED | 14d51f0 | |
| 0 | transport-commit | a3s_transport | — | MERGED | 7770850 | download side, live WOG test |
| 0 | descriptor-lock | a3s_types tests | — | MERGED | 575c386 | |
| 0 | WP01 | java_fixture_harness | — | BLOCKED | | needs JDK 25 |
| 0 | WP02 | utils crates | — | IMPL | | |
| 0 | WP03 | a3s_types local | — | IMPL | | wire-touching |
| 0 | WP06 | a3s_transport upload | transport-commit | IMPL | | |
| 0 | WP17 | a3s_tools bikey/rpt/radio | — | SPEC_OK | | |
| 0 | WP20 | a3s_ui | — | SPEC_OK | | |
| 0 | WP31 | a3s_updater core | — | SPEC_OK | | |
| 1 | WP04 | a3s_repo_files local/legacy | WP02, WP03 | SPEC_OK | | |
| 1 | WP05 | autoconfig url/export | WP03 | SPEC_OK | | |
| 1 | WP07 | a3s_sync model/remote/status | WP02, WP03 | SPEC_OK | | critical path |
| 1 | WP11 | a3s_admin build | WP03 | SPEC_OK | | wire-touching |
| 1 | WP13 | a3s_addons | WP03 | SPEC_OK | | |
| 1 | WP15 | a3s_platform | WP02 | SPEC_OK | | |
| 2 | WP08 | a3s_sync plan | WP07 | SPEC_OK | | |
| 2 | WP09 | a3s_sync download | WP07 | SPEC_OK | | |
| 2 | WP12 | a3s_admin upload/check/events | WP06, WP07, WP11 | SPEC_OK | | split WP12a → WP12b; Q7 needs user |
| 2 | WP14 | app services | WP04, WP05 | SPEC_OK | | |
| 2 | WP16 | a3s_launch | WP13, WP15 | SPEC_OK | | |
| 2 | WP18 | presets | WP13, WP02, WP04 | SPEC_OK | | |
| 2 | WP32 | updater apply | WP31, WP15 | SPEC_OK | | |
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
- D-7 2026-10-10: Rust config/data dir name `arma3sync-rs` (user).
- D-8 2026-10-10: orchestrator may approve local-only deviations that fix Java bugs/security; wire-visible changes go to the user (user).
- D-9 2026-10-10: custom agent types need a session restart; until then spawn general-purpose/opus with a ROLE header. (Now registered in this session.)

## Follow-ups

- F-1 WP03: confirm PROVISIONAL computed UID of `domain.AutoConfig` (and FileAttributes, configration.Proxy) with the JDK harness (WP01).
- F-3 After WP06: `a3s_transport::is_local_test_host` delegates to `a3s_types::ssl_policy` (WP05 Q3).
- F-4 OS keyring for repository passwords (WP04 Q1; TOML is plain text, 0600).
- F-5 Optional `HttpTransport::probe_range` returning headers for the partial-transfer test dialog (WP07 Q2).
- F-6 Manual check: Linux `steam -applaunch 107410 -noLauncher` launch path and Steam argument prompt (WP16 A21).
- F-7 SOCKS proxy for FTP/SFTP in a3s_transport (WP14 Q8).
- F-8 Shared in-process test-server crate (FTP/SFTP/WebDAV/HTTP) instead of #[path] includes (WP12 Q2).
- F-2 CI: add mesa Vulkan drivers to rust.yml for egui_kittest snapshot tests (WP20 Q10).

## Live test log

| Date | Crate | Test | Result |
| --- | --- | --- | --- |
| 2026-10-10 | a3s_transport | live_wog_ftp | FAIL-LOGIC: FTP resume got whole file (REST before PASV); full download + SHA-1 OK. Routed to WP06. |

## Last reports
