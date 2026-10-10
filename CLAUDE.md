# Notes for Claude

* Rust rewrite of the Java Arma3Sync. Java reference (read-only, gitignored):
  `/run/media/system/Main_storage/Projects/arma_3_sync_rs/java_version/Arma3Sync/src/main/java/fr/soe/a3s/`
* Layout and wire-format contract: `ARCHITECTURE.md`. Style: `CODE_STYLE.md`.
* Remaining work is driven by sub-agents: loop in `docs/ORCHESTRATION.md`, state in
  `docs/PROGRESS.md`, packages in `docs/WORK_PACKAGES.md`, specs in `docs/specs/`.
  Resume with `/orchestrate`.
* Run cargo through `scripts/gate.sh` (per-slot target dirs; agents in worktrees must).
  Gates: `scripts/gate.sh crate <crate>`, `scripts/gate.sh workspace`, `scripts/gate.sh golden`.
* Never change Java class names, serialVersionUIDs or field descriptors in `a3s_types`
  (`crates/store/a3s_types/tests/descriptor_lock.rs` guards them).
* Live test repository (read-only, anonymous FTP, 93 GB — never download in full):
  `ftp://88.99.69.243/Repo/WOG/.a3s/autoconfig`, via `scripts/gate.sh live <crate>` only.
  `https://repo.ofcra.org` is unreachable from this machine; test HTTP/zsync with local servers.
