//! Installing a downloaded release archive: what kind of installation can be updated, what a
//! release archive must contain, and the install itself.

use std::fmt;
use std::path::Path;

use a3s_foundation::paths::{Os, PORTABLE_DATA_DIR, PORTABLE_MARKER};

use super::ApplyError;
use super::replace::{STAGE_PREFIX, cleanup_leftovers};
use crate::{
    DiagnosticLog, FileReplacer, InstallReport, StagedEntry, UpdateError, install_staged,
    stage_archive, validate_archive,
};

/// File name of the application binary at the root of a release archive and of an
/// installation: `arma3sync.exe` on Windows, `arma3sync` elsewhere.
pub fn release_binary_name(os: Os) -> &'static str {
    if os == Os::Windows {
        "arma3sync.exe"
    } else {
        "arma3sync"
    }
}

/// How the running application was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// A plain directory holding the binary: the only kind the updater replaces.
    Directory,

    /// A Linux `AppImage` (`APPIMAGE` is set).
    AppImage,

    /// A macOS `.app` bundle (`*.app/Contents/MacOS/<binary>`).
    MacBundle,

    /// The directory of the Java client (it holds `ArmA3Sync-Updater.jar`), whose files the
    /// Rust release would delete.
    JavaInstallation,

    /// A cargo build directory (`target/debug` or `target/release`).
    Development,
}

impl fmt::Display for InstallKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Directory => "installation directory",
            Self::AppImage => "AppImage",
            Self::MacBundle => "macOS app bundle",
            Self::JavaInstallation => "Java ArmA3Sync installation",
            Self::Development => "development build",
        })
    }
}

/// Classifies the installation of `exe_path`.
///
/// The first matching rule wins: `AppImage` (Linux with a non-blank `appimage_env`), macOS bundle, Java installation, cargo build directory,
/// else a plain directory.
pub fn detect_install_kind(exe_path: &Path, os: Os, appimage_env: Option<&str>) -> InstallKind {
    if os == Os::Linux && appimage_env.is_some_and(|value| !value.trim().is_empty()) {
        return InstallKind::AppImage;
    }
    let Some(dir) = exe_path.parent() else {
        return InstallKind::Directory;
    };
    fn name(path: Option<&Path>) -> Option<std::borrow::Cow<'_, str>> {
        path.and_then(Path::file_name).map(|n| n.to_string_lossy())
    }
    let contents = dir.parent();
    let bundle = contents.and_then(Path::parent);
    if name(Some(dir)).as_deref() == Some("MacOS")
        && name(contents).as_deref() == Some("Contents")
        && name(bundle).is_some_and(|n| n.to_ascii_lowercase().ends_with(".app"))
    {
        return InstallKind::MacBundle;
    }
    if dir.join("ArmA3Sync-Updater.jar").is_file() {
        return InstallKind::JavaInstallation;
    }
    if dir.join(".cargo-lock").is_file() {
        return InstallKind::Development;
    }
    InstallKind::Directory
}

/// Checks a staged release before anything is written.
///
/// Exactly one entry must be the platform binary
/// at the archive root (ASCII case-insensitive on Windows), and no entry is the portable-mode
/// marker or lies in the portable data folder. On Unix the binary is made executable: a mode
/// without the owner execute bit (or none, as in archives built on Windows) becomes `0o755`.
pub fn check_release_layout(entries: &mut [StagedEntry], os: Os) -> Result<(), String> {
    let binary = release_binary_name(os);
    let is_binary = |relative: &str| {
        if os == Os::Windows {
            relative.eq_ignore_ascii_case(binary)
        } else {
            relative == binary
        }
    };
    if entries.iter().filter(|e| is_binary(&e.relative)).count() != 1 {
        return Err(format!("Update archive does not contain {binary}."));
    }
    if let Some(entry) = entries.iter().find(|e| is_portable_data(&e.relative)) {
        return Err(format!(
            "Update archive contains user data path: {}",
            entry.relative
        ));
    }
    if os != Os::Windows {
        for entry in entries.iter_mut().filter(|e| is_binary(&e.relative)) {
            entry.unix_mode = Some(match entry.unix_mode {
                Some(mode) if mode & 0o100 != 0 => mode & 0o777,
                _ => 0o755,
            });
        }
    }
    Ok(())
}

fn is_portable_data(relative: &str) -> bool {
    let top = relative.split('/').next().unwrap_or_default();
    relative.eq_ignore_ascii_case(PORTABLE_MARKER) || top.eq_ignore_ascii_case(PORTABLE_DATA_DIR)
}

/// Installs a downloaded release archive into `install_dir`; the in-process update and the
/// elevated updater both use it.
///
/// 1. Removes leftovers of earlier updates ([`cleanup_leftovers`]).
/// 2. Copies the archive privately while hashing it and validates it ([`validate_archive`]);
///    a different SHA-256 than `expected_sha256` fails with
///    `ApplyError::Download(UpdateError::Integrity)`.
/// 3. Extracts it into a new `.a3s-stage-<pid>-*` directory inside `install_dir`, removed on return.
///    The installation is the one place an elevated updater may write that a less-privileged
///    user cannot, so nobody can swap the staged files before they are copied.
/// 4. Checks the release layout ([`check_release_layout`]); nothing in the installation has
///    changed yet.
/// 5. Copies the files with `replacer` ([`install_staged`]).
// Mirrors a3sUpdater/service/Service.java:158-175
pub fn install_verified_archive(
    archive: &Path,
    expected_sha256: &str,
    install_dir: &Path,
    os: Os,
    replacer: &dyn FileReplacer,
    diag: &DiagnosticLog,
) -> Result<InstallReport, ApplyError> {
    cleanup_leftovers(install_dir);
    diag.info(&format!("Installing update into {}", install_dir.display()));
    let install_error = |source: UpdateError| {
        let err = ApplyError::Install {
            dir: install_dir.to_path_buf(),
            source,
        };
        diag.error("Update installation copy failed.", Some(&err));
        err
    };
    let stage = |stage: &'static str, source: UpdateError| UpdateError::Stage {
        stage,
        source: Box::new(source),
    };

    let mut verified = match validate_archive(archive, Some(expected_sha256)) {
        Ok(verified) => verified,
        Err(UpdateError::Integrity) => return Err(ApplyError::Download(UpdateError::Integrity)),
        Err(err) => return Err(install_error(stage("archive validation", err))),
    };
    let staging = tempfile::Builder::new()
        .prefix(&format!("{STAGE_PREFIX}{}-", std::process::id()))
        .tempdir_in(install_dir)
        .map_err(|e| {
            let err = UpdateError::Io {
                context: format!(
                    "Could not create a staging directory in {}",
                    install_dir.display()
                ),
                source: e,
            };
            install_error(stage("archive extraction", err))
        })?;
    let mut entries = stage_archive(&mut verified, staging.path())
        .map_err(|e| install_error(stage("archive extraction", e)))?;
    check_release_layout(&mut entries, os).map_err(ApplyError::Layout)?;
    let report = install_staged(install_dir, &entries, replacer)
        .map_err(|e| install_error(stage("installation copy", e)))?;
    diag.info("Update installation copy completed.");
    Ok(report)
}
