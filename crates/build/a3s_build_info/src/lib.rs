//! Build-time information (version, git hash, target) embedded into binaries.
//!
//! Port of the Java client's `main/Version.java`. The version comes from the workspace
//! `Cargo.toml` (semver, e.g. `0.1.0-alpha.1`) instead of Java's calendar version; the git hash,
//! target triple and profile are baked in by `build.rs`.

use chrono::Datelike as _;

/// Application version, the single version source (`CARGO_PKG_VERSION`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git hash of the build, overridable with the `A3S_GIT_HASH` environment variable at
/// build time, `unknown` when git is unavailable.
pub const GIT_HASH: &str = env!("A3S_BUILD_GIT_HASH");

/// Target triple the binary was built for.
pub const TARGET: &str = env!("A3S_BUILD_TARGET");

/// Cargo profile of the build (`debug` or `release`).
pub const PROFILE: &str = env!("A3S_BUILD_PROFILE");

/// Year of the first Arma3Sync release, start of the copyright range.
// Mirrors main/Version.java:12
pub const FIRST_RELEASE_YEAR: i32 = 2013;

/// Short version name, `major.minor` (`0.1` for `0.1.0-alpha.1`).
// Mirrors main/Version.java:18-21
pub fn name() -> String {
    format!(
        "{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR")
    )
}

/// Build number: the semver patch number.
///
/// Java returns `0` when the third part is not a number; Rust uses the numeric patch.
// Mirrors main/Version.java:23-31
pub fn build() -> u32 {
    env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0)
}

/// Copyright year range: `2013` in 2013, else `2013-{current_year}` (not clamped, as in Java).
// Mirrors main/Version.java:33-38
pub fn copyright_years(current_year: i32) -> String {
    if current_year == FIRST_RELEASE_YEAR {
        FIRST_RELEASE_YEAR.to_string()
    } else {
        format!("{FIRST_RELEASE_YEAR}-{current_year}")
    }
}

/// [`copyright_years`] for the current year of the local calendar.
pub fn copyright_years_now() -> String {
    copyright_years(chrono::Local::now().year())
}

/// Two-line version banner of the console: `ArmA3Sync version {name}` and
/// `Build {version} ({years})`.
// Mirrors console/CommandConsole.java:744-745
pub fn version_banner() -> String {
    format!(
        "ArmA3Sync version {}\nBuild {VERSION} ({})",
        name(),
        copyright_years_now()
    )
}

/// Startup line `Arma3Sync Installed version = {version}`.
// Mirrors main/ArmA3Sync.java:90
pub fn installed_version_line() -> String {
    format!("Arma3Sync Installed version = {VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_b1_copyright_year_ends_with_current_year() {
        let year = chrono::Local::now().year();
        assert!(copyright_years_now().ends_with(&year.to_string()));
    }

    #[test]
    fn a_b2_copyright_years() {
        assert_eq!(copyright_years(2013), "2013");
        assert_eq!(copyright_years(2026), "2013-2026");
        assert_eq!(copyright_years(2010), "2013-2010");
    }

    #[test]
    fn a_b3_version_fields() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            name(),
            format!(
                "{}.{}",
                env!("CARGO_PKG_VERSION_MAJOR"),
                env!("CARGO_PKG_VERSION_MINOR")
            )
        );
        assert_eq!(
            build().to_string(),
            env!("CARGO_PKG_VERSION_PATCH"),
            "build() is the numeric patch"
        );
        assert!(!GIT_HASH.is_empty());
        assert!(!TARGET.is_empty());
        assert!(!PROFILE.is_empty());
    }

    #[test]
    fn a_b3_banner_shapes() {
        let banner = version_banner();
        let lines: Vec<&str> = banner.lines().collect();
        assert_eq!(lines.len(), 2, "{banner:?}");
        assert_eq!(lines[0], format!("ArmA3Sync version {}", name()));
        assert_eq!(
            lines[1],
            format!("Build {VERSION} ({})", copyright_years_now())
        );
        assert_eq!(
            installed_version_line(),
            format!("Arma3Sync Installed version = {VERSION}")
        );
    }
}
