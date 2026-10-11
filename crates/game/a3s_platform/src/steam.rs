//! Steam installations, their libraries (`libraryfolders.vdf`), installed apps
//! (`appmanifest_<id>.acf`) and Proton prefixes. Java had none of this: it only read the Arma 3
//! registry key on Windows.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_foundation::paths::{Os, PlatformEnv};

use crate::detect::steam_dir_from_registry;
use crate::registry::RegistryRead;
use crate::vdf::{self, Value};

/// Steam app id of Arma 3.
pub const ARMA3_APP_ID: u32 = 107_410;

/// Install folder name Steam uses for Arma 3 when a manifest has no `installdir`.
const ARMA3_DEFAULT_INSTALL_DIR: &str = "Arma 3";

/// Largest Steam metadata file read (the VDF parser's own limit).
const MAX_METADATA_BYTES: u64 = 1 << 20;

/// An app installed in a Steam library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamApp {
    /// The Steam app id.
    pub app_id: u32,

    /// The library folder holding the app (the folder containing `steamapps`).
    pub library: PathBuf,

    /// The app's install directory, `<library>/steamapps/common/<installdir>`.
    pub install_dir: PathBuf,
}

/// The Arma 3 folders inside an app's Proton (Wine) prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtonPaths {
    /// The prefix, `<library>/steamapps/compatdata/<app id>/pfx`.
    pub prefix: PathBuf,

    /// The prefix's equivalent of `Documents\Arma 3` (profiles, `Arma3.cfg`).
    pub documents_arma3: PathBuf,

    /// The prefix's equivalent of `%LOCALAPPDATA%\Arma 3` (RPT logs).
    pub local_appdata_arma3: PathBuf,
}

/// The existing Steam installation roots, most likely first, without duplicates.
///
/// Windows: the registry install path, then `Steam` under `ProgramFiles(x86)` and
/// `ProgramFiles`. Linux and BSD: `$XDG_DATA_HOME/Steam`, `~/.local/share/Steam`, `~/.steam/root`,
/// `~/.steam/steam`, the Flatpak and the Snap locations. macOS: `~/Library/Application
/// Support/Steam`.
pub fn steam_roots(env: &PlatformEnv, registry: &dyn RegistryRead) -> Vec<PathBuf> {
    let home = env.home();
    let in_home = |relative: &str| home.map(|home| home.join(relative));
    let candidates: Vec<Option<PathBuf>> = match env.os() {
        Os::Windows => vec![
            steam_dir_from_registry(env, registry),
            env.var("ProgramFiles(x86)")
                .map(|dir| Path::new(dir).join("Steam")),
            env.var("ProgramFiles")
                .map(|dir| Path::new(dir).join("Steam")),
        ],
        Os::Linux | Os::OtherUnix => vec![
            env.var("XDG_DATA_HOME")
                .map(Path::new)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("Steam")),
            in_home(".local/share/Steam"),
            in_home(".steam/root"),
            in_home(".steam/steam"),
            in_home(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            in_home("snap/steam/common/.local/share/Steam"),
        ],
        Os::MacOs => vec![in_home("Library/Application Support/Steam")],
        Os::Other => Vec::new(),
    };
    existing_unique_dirs(candidates.into_iter().flatten())
}

/// The libraries of a Steam root, existing directories only and without duplicates.
///
/// The root itself comes first, then the libraries listed in its `steamapps/libraryfolders.vdf`.
/// A missing or unreadable file leaves just the root.
pub fn steam_libraries(root: &Path) -> Vec<PathBuf> {
    let file = root.join("steamapps").join("libraryfolders.vdf");
    let listed = read_metadata(&file)
        .map(|text| parse_library_folders(&text))
        .unwrap_or_default();
    existing_unique_dirs(std::iter::once(root.to_path_buf()).chain(listed))
}

/// The library paths listed in a `libraryfolders.vdf`, in file order.
///
/// Understands the current format (`"libraryfolders" { "0" { "path" "…" … } }`) and the legacy one
/// (`"LibraryFolders" { "1" "D:\\SteamLibrary" }`). An unparsable document lists nothing.
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let entries = match vdf::parse(text) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!("cannot parse libraryfolders.vdf: {err}");
            return Vec::new();
        }
    };
    let Some(folders) = vdf::get_obj(&entries, "libraryfolders") else {
        return Vec::new();
    };
    folders
        .iter()
        .filter_map(|(key, value)| match value {
            Value::Obj(library) => vdf::get_str(library, "path"),
            Value::Str(path) if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) => {
                Some(path.as_str())
            }
            Value::Str(_) => None,
        })
        .map(PathBuf::from)
        .collect()
}

/// The `AppState.installdir` of an `appmanifest_<id>.acf`, if present and parsable.
pub fn parse_install_dir(acf: &str) -> Option<String> {
    parse_manifest(acf)?.install_dir
}

/// The parts of an app manifest this crate uses.
struct Manifest {
    install_dir: Option<String>,
}

/// `None` when the manifest is unparsable or has no `AppState`.
fn parse_manifest(acf: &str) -> Option<Manifest> {
    let entries = vdf::parse(acf)
        .map_err(|err| tracing::debug!("cannot parse an app manifest: {err}"))
        .ok()?;
    let state = vdf::get_obj(&entries, "AppState")?;
    Some(Manifest {
        install_dir: vdf::get_str(state, "installdir").map(str::to_owned),
    })
}

/// The first library in `roots` (in order) where `app_id` is installed: its manifest parses and
/// its `steamapps/common/<installdir>` folder exists.
pub fn find_steam_app(roots: &[PathBuf], app_id: u32) -> Option<SteamApp> {
    let mut libraries = Vec::new();
    for root in roots {
        libraries.extend(steam_libraries(root));
    }
    existing_unique_dirs(libraries)
        .into_iter()
        .find_map(|library| app_in_library(library, app_id))
}

fn app_in_library(library: PathBuf, app_id: u32) -> Option<SteamApp> {
    let steamapps = library.join("steamapps");
    let manifest = read_metadata(&steamapps.join(format!("appmanifest_{app_id}.acf")))?;
    let install_dir_name = match parse_manifest(&manifest)?.install_dir {
        Some(name) => name,
        None if app_id == ARMA3_APP_ID => ARMA3_DEFAULT_INSTALL_DIR.to_owned(),
        None => return None,
    };
    // The name is one folder below `common`; anything else is not a Steam-written manifest.
    if install_dir_name.is_empty()
        || install_dir_name == "."
        || install_dir_name == ".."
        || install_dir_name.contains(['/', '\\'])
    {
        return None;
    }
    let install_dir = steamapps.join("common").join(install_dir_name);
    install_dir.is_dir().then_some(SteamApp {
        app_id,
        library,
        install_dir,
    })
}

/// Arma 3 in the Steam libraries of this machine.
pub fn find_arma3(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<SteamApp> {
    find_steam_app(&steam_roots(env, registry), ARMA3_APP_ID)
}

/// The Proton prefix folders of `app`, if the app has a prefix. The returned folders may not
/// exist.
pub fn proton_paths(app: &SteamApp) -> Option<ProtonPaths> {
    let prefix = app
        .library
        .join("steamapps")
        .join("compatdata")
        .join(app.app_id.to_string())
        .join("pfx");
    if !prefix.is_dir() {
        return None;
    }
    let user = prefix.join("drive_c").join("users").join("steamuser");
    // Older Proton versions use the Windows XP name.
    let documents = if user.join("Documents").exists() {
        user.join("Documents")
    } else {
        user.join("My Documents")
    };
    Some(ProtonPaths {
        documents_arma3: documents.join("Arma 3"),
        local_appdata_arma3: user.join("AppData").join("Local").join("Arma 3"),
        prefix,
    })
}

/// Reads a Steam metadata file; `None` if it is missing, too large or not UTF-8.
fn read_metadata(path: &Path) -> Option<String> {
    let mut text = String::new();
    let read = fs::File::open(path)
        .and_then(|file| file.take(MAX_METADATA_BYTES + 1).read_to_string(&mut text));
    match read {
        Ok(_) => Some(text),
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!("cannot read {}: {err}", path.display());
            }
            None
        }
    }
}

/// The existing directories among `candidates`, in order, without duplicates (compared after
/// resolving symbolic links).
fn existing_unique_dirs(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut dirs = Vec::new();
    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }
        let identity = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen.contains(&identity) {
            seen.push(identity);
            dirs.push(dir);
        }
    }
    dirs
}
