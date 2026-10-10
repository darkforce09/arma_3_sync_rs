//! Names and extensions used in repository layouts (`dao/DataAccessConstants.java`).

/// Metadata folder at the root of every repository.
pub const A3S_FOLDER: &str = ".a3s";

/// Sibling file holding zsync block checksums, for HTTP repositories.
pub const ZSYNC_EXTENSION: &str = ".zsync";

/// Sibling file holding a compressed copy of a `.pbo` / `.ebo`, for compressed repositories.
pub const ZIP_EXTENSION: &str = ".zip";

/// Temporary file used while patching with zsync.
pub const PART_EXTENSION: &str = ".part";

/// Local client file holding an (encrypted) repository configuration.
pub const REPOSITORY_EXTENSION: &str = ".a3s.repository";

/// Local client profile file.
pub const PROFILE_EXTENSION: &str = ".a3s.profile";

/// Exported auto-configuration file.
pub const AUTOCONFIG_EXPORT_EXTENSION: &str = ".a3s.autoconfig";

/// Whether the repository builder skips a file with this name
/// (`RepositoryBuildProcessor.generateSync`: names *containing* these markers).
pub fn is_builder_ignored_file(name: &str) -> bool {
    name.contains(ZSYNC_EXTENSION) || name.contains(".pbo.zip") || name.contains(".a3s")
}

/// Whether a file is compressed into `<name>.zip` in compressed repositories.
pub fn is_compressible(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".pbo") || lower.ends_with(".ebo")
}
