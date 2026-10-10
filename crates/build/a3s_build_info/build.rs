//! Bakes the git hash, target triple and profile into `a3s_build_info`.
//!
//! The build must succeed without git and outside a git checkout: the hash then falls back to
//! `unknown`. No timestamp is embedded, so builds stay reproducible.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=A3S_GIT_HASH");
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());

    let git_hash = env::var("A3S_GIT_HASH")
        .ok()
        .map(|hash| hash.trim().to_owned())
        .filter(|hash| !hash.is_empty())
        .or_else(|| git(&manifest_dir, &["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=A3S_BUILD_GIT_HASH={git_hash}");

    for (source, key) in [
        ("TARGET", "A3S_BUILD_TARGET"),
        ("PROFILE", "A3S_BUILD_PROFILE"),
    ] {
        let value = env::var(source).unwrap_or_else(|_| "unknown".to_owned());
        println!("cargo:rustc-env={key}={value}");
    }

    rerun_if_head_changes(&manifest_dir);
}

/// Emits `rerun-if-changed` for `HEAD`, the ref it points at and `packed-refs` (where git
/// moves refs on `git pack-refs`/`gc`), when those files exist.
fn rerun_if_head_changes(manifest_dir: &Path) {
    let Some(head) = git_path(manifest_dir, "HEAD") else {
        return;
    };
    if !head.is_file() {
        return;
    }
    println!("cargo:rerun-if-changed={}", head.display());

    if let Some(packed_refs) = git_path(manifest_dir, "packed-refs")
        && packed_refs.is_file()
    {
        println!("cargo:rerun-if-changed={}", packed_refs.display());
    }

    let Ok(contents) = std::fs::read_to_string(&head) else {
        return;
    };
    if let Some(reference) = contents.trim().strip_prefix("ref:")
        && let Some(ref_path) = git_path(manifest_dir, reference.trim())
        && ref_path.is_file()
    {
        println!("cargo:rerun-if-changed={}", ref_path.display());
    }
}

/// Path of a file inside the git directory (handles worktrees), relative paths resolved against
/// `manifest_dir`.
fn git_path(manifest_dir: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git(manifest_dir, &["rev-parse", "--git-path", name])?);
    Some(if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    })
}

/// Runs git in `dir`, returning its trimmed stdout on success.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
