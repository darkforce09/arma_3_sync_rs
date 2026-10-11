# Rewrite progress

Mode: PAUSED (user restarting PC; resume with /orchestrate)            Updated: 2026-10-11
Current wave: 0/1          Next action: on resume — check WP11 worktree (.claude/worktrees/agent-a39b7a5c85d46a5e2): if its agent is gone, spawn a new rust-implementer told to `git merge --ff-only worktree-agent-a39b7a5c85d46a5e2` in its fresh worktree and finish WP11 fix round 1 (items listed in docs/specs/WP11.md 2026-10-11 lines + the fix-round message: Java ref-count limit via a3s_jserial reader exception, cache commit after zip/zsync, umask perms, .a3s.tmp temp names, D-17, ZIP64 margin, minors). Then: start WP10 (sync pipeline, deps merged), WP14 needs WP05 → start WP05, WP13, WP15; then WP21.
Waiting-on: WP11 fix round 1 (interrupted; WIP commit 3cdce4f — item 1 partly done: java_filter_stats in a3s_jserial + encode_checked; items 2–6 + minors remain; serde_json 1.0.152 bump is fine). gate.sh now sources scripts/java_env.sh automatically.
Blockers: none

## In flight

| WP | Crate | State | Round | Agents | Slot | Branch | Worktree | HEAD | Since |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-11 | a3s_sync (WP09 branch) | a17_live_wog_download (≤3 files, ≤1 MB) | PASS |
| 2026-10-11 | a3s_sync (WP07 branch) | a19_live_wog_check, import_autoconfig WOG | PASS |
| WP11 | a3s_admin build | FIX round 1 (Java ref-count limit, cache commit, umask, temp names, D-17; Java reads Rust PASS) | 1 | impl=a39b7a5c ver=aee386a7 rev=a4b331ee | 3 | worktree-agent-a39b7a5c85d46a5e2 | .claude/worktrees/agent-a39b7a5c85d46a5e2 | 504cd66 | 2026-10-11 |

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
| 0 | WP01 | java_fixture_harness | — | MERGED | | JDK 25 via scripts/java_env.sh; javac only, commons-io stub |
| 0 | WP02 | utils crates | — | MERGED | (see git log) | |
| 0 | WP03 | a3s_types local | — | MERGED | cea732e | wire-touching |
| 0 | WP06 | a3s_transport upload | transport-commit | MERGED | 9dc241f | |
| 0 | WP17 | a3s_tools bikey/rpt/radio | — | SPEC_OK | | |
| 0 | WP20 | a3s_ui | — | MERGED | | |
| 0 | WP31 | a3s_updater core | — | SPEC_OK | | |
| 1 | WP04 | a3s_repo_files local/legacy | WP02, WP03 | MERGED | | |
| 1 | WP05 | autoconfig url/export | WP03 | SPEC_OK | | |
| 1 | WP07 | a3s_sync model/remote/status | WP02, WP03 | MERGED | (git log) | | critical path |
| 1 | WP11 | a3s_admin build | WP03 | VERIFY | | wire-touching |
| 1 | WP13 | a3s_addons | WP03 | SPEC_OK | | |
| 1 | WP15 | a3s_platform | WP02 | SPEC_OK | | |
| 2 | WP08 | a3s_sync plan | WP07 | MERGED | | |
| 2 | WP09 | a3s_sync download | WP07 | MERGED | | |
| 2 | WP12 | a3s_admin upload/check/events | WP06, WP07, WP11 | SPEC_OK | | split WP12a → WP12b |
| 2 | WP14 | app services | WP04, WP05 | SPEC_OK | | |
| 2 | WP16 | a3s_launch | WP13, WP15 | SPEC_OK | | |
| 2 | WP18 | presets | WP13, WP02, WP04 | SPEC_OK | | |
| 2 | WP32 | updater apply | WP31, WP15 | SPEC_OK | | |
| 3 | WP10 | sync pipeline | WP08, WP09 | SPEC_OK | | |
| 3 | WP21 | a3s_viewer_context | WP14, WP07 | SPEC_OK | | |
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
- D-11 2026-10-10: WP12 Q7 — fix nested `racine` folder upload paths (user).
- D-12 2026-10-10: deny.toml ignores RUSTSEC-2023-0071 (rsa Marvin, private-key ops only; we only verify SFTP host keys). Re-evaluate if SFTP public-key auth is added.
- D-13 2026-10-11: a3s_ui indeterminate mark = accent_primary, unchecked border = text_disabled, mode-aware JAVA_BLUE helper (readability on dark; local-only).
- D-14 2026-10-11: Rust builder excludes .ebo.zip (and any compression-stage .zip) from the sync tree; Java only excludes .pbo.zip (user).
- D-15 2026-10-11: sync plan rejects unsafe remote names (path traversal, absolute paths, separators) and never follows symlinked dirs in the extra-file scan (security hardening; Java is vulnerable).
- D-16 2026-10-11: downloads follow user-created symlinks inside the destination; only remote names are validated (no .., absolute, separators). Deletions never follow links.
- D-17 2026-10-11: symlinked repository root → real totalFilesSize (user).
- D-10 2026-10-10: JDK = user-local Temurin 25.0.4.1 (Bazzite, no dnf); harness sources scripts/java_env.sh; no Gradle — compile with javac.
- D-9 2026-10-10: custom agent types need a session restart; until then spawn general-purpose/opus with a ROLE header. (Now registered in this session.)

## Follow-ups

- F-1 RESOLVED by WP01 spec measurements (UIDs confirmed on JDK 25); fixture tests will pin them.
- F-3 After WP06: `a3s_transport::is_local_test_host` delegates to `a3s_types::ssl_policy` (WP05 Q3).
- F-4 OS keyring for repository passwords (WP04 Q1; TOML is plain text, 0600).
- F-5 Optional `HttpTransport::probe_range` returning headers for the partial-transfer test dialog (WP07 Q2).
- F-6 Manual check: Linux `steam -applaunch 107410 -noLauncher` launch path and Steam argument prompt (WP16 A21).
- F-7 SOCKS proxy for FTP/SFTP in a3s_transport (WP14 Q8).
- F-8 Shared in-process test-server crate (FTP/SFTP/WebDAV/HTTP) instead of #[path] includes (WP12 Q2).
- F-9 WP02 review MINORs: a3s_log invalid-A3S_LOG warning before the "Debug logging enabled" line; rotating file never reopens after a failed reopen; PlatformEnv::current fails when current_exe fails (make exe_dir optional); Os mapping of SunOS/AIX should be Other; legacy normalize canonical-vs-lexical mismatch; CreateDir message should use an absolute path; remove redundant crates/utils/a3s_error/_typos.toml; build.rs rerun-if-changed for packed-refs.
- F-10 WP03 review MINORs: export tree depth > ~47 groups exceeds Java's reader depth limit (document or error); skipped invalid export elements = approved deviation (D-8).
- F-11 suppaftp trace-logs `PASS <password>` via the `log` crate: keep `suppaftp` target at debug or above in app log filters (WP06 review).
- F-12 WP06 re-review MINORs: fix webdav.rs:207 Mirrors comment (deviation); CWD-back refusal discards listing (ftp.rs:564); Unusable listing makes refused DELE look Ok (ftp.rs:765/815); skip CWD before STOR when dir unchanged (perf); MLST fallback trailing slash.
- F-13 WP07 parity MINORs: Java texts for FTP missing remote dir and unsupported protocol on connect; HTTP status text 'Server response code is N Reason'; cancel during sync decode keeps decoded slot.
- F-14 a3s_transport: FTP control replies (retr_as_stream / completion) are not cancel-aware; make every FTP command race the CancellationToken (WP07 verifier).
- F-15 (fixtures now on main) WP03 PROVISIONAL UID markers can be lifted (WP01 verifier: all 25 descriptors + 3 computed UIDs match JDK 25).
- F-16 a3s_ui dialog focus MINORs: popup press steals dialog focus; focus not handed back when focused dialog closes; Escape not gated by egui_wants_keyboard_input when focus moved by Tab; modal keys only from 2nd frame (document).
- F-17 a3s_sync: expose RemoteConnection::fetch_file (poisons + B13 error) and use it from completion.rs; run classify_leaves off the async thread.
- F-18 WP08 MINORs: matched remote folder that is a local symlink is still listed (doc says never followed — narrow doc or skip); completion take() empties tree on future drop (document/clone); reject Windows trailing dot/space and reserved device names; symlinked dir delete needs remove_dir on Windows (WP10); validate_names runs twice.
- F-19 WP09 MINORs: watch_effective_offset single probe; a6 resume test can't prove offset correction; tests for 416-on-resume, WebDAV no-resume, worker panic → Failed. reqwest dep in a3s_sync approved post hoc.
- F-20 WP04 MINORs: unreadable non-canonical profile/repo files don't block duplicates; Default fallback should persist profile_name=Default; inner sealed-stream Types error → Java 'Failed to decrypt serialized file'.
- F-2 CI: add mesa Vulkan drivers to rust.yml for egui_kittest snapshot tests (WP20 Q10).

## Live test log

| Date | Crate | Test | Result |
| --- | --- | --- | --- |
| 2026-10-10 | a3s_transport (WP06 round 1) | live_wog_ftp, resume, session exists CWD+LIST | PASS |
| 2026-10-10 | a3s_transport (WP06 branch) | live_wog_ftp + verify_live resume | PASS: PASV→REST→RETR honoured by WOG |
| 2026-10-10 | a3s_transport | live_wog_ftp | FAIL-LOGIC: FTP resume got whole file (REST before PASV); full download + SHA-1 OK. Routed to WP06. |

## Last reports
