//! Steam metadata parsing, library and app discovery in a fake home, and Proton prefixes.

use std::fs;
use std::path::{Path, PathBuf};

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_platform::detect::{detect_arma3_dir, rpt_dir};
use a3s_platform::folders::{MemoryFolders, arma3_documents_dir};
use a3s_platform::registry::MemoryRegistry;
use a3s_platform::steam::{
    ARMA3_APP_ID, SteamApp, find_arma3, parse_install_dir, parse_library_folders, proton_paths,
    steam_roots,
};

const MODERN_LIBRARY_FOLDERS: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
		"apps"
		{
			"228980"		"0"
		}
	}
	// the second drive
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"107410"		"45000000000"
		}
	}
}
"#;

const ARMA3_MANIFEST: &str = r#"
"AppState"
{
	"appid"		"107410"
	"name"		"Arma 3"
	"InstallDir"		"Arma 3"
}
"#;

#[test]
fn modern_library_folders_list_paths_in_order() {
    assert_eq!(
        parse_library_folders(MODERN_LIBRARY_FOLDERS),
        vec![
            PathBuf::from("/home/u/.local/share/Steam"),
            PathBuf::from(r"D:\SteamLibrary"),
        ]
    );
}

#[test]
fn legacy_library_folders_list_numbered_values() {
    let legacy = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1580000000"
	"ContentStatsID"		"-123"
	"1"		"D:\\SteamLibrary"
}
"#;
    assert_eq!(
        parse_library_folders(legacy),
        vec![PathBuf::from(r"D:\SteamLibrary")]
    );
}

#[test]
fn unquoted_tokens_and_conditionals_are_accepted() {
    let text = r#"libraryfolders { 0 { path /mnt/games [$LINUX] apps { 107410 1 } } }"#;
    assert_eq!(
        parse_library_folders(text),
        vec![PathBuf::from("/mnt/games")]
    );
}

#[test]
fn malformed_documents_list_nothing() {
    assert!(parse_library_folders(r#""libraryfolders" { "0" { "path" "/x" }"#).is_empty());
    assert!(parse_library_folders(r#""libraryfolders" { } }"#).is_empty());
    assert!(parse_library_folders(r#""libraryfolders" { "path" "#).is_empty());

    let nested = |depth: usize| {
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"/ok\" }} {}{} }}",
            "\"k\" { ".repeat(depth - 1),
            "} ".repeat(depth - 1)
        )
    };
    assert_eq!(
        parse_library_folders(&nested(32)),
        vec![PathBuf::from("/ok")]
    );
    assert!(parse_library_folders(&nested(33)).is_empty());

    let padding = " ".repeat(1 << 20);
    let oversized = format!(r#""libraryfolders" {{ "1" "/ok" }}{padding}"#);
    assert!(parse_library_folders(&oversized).is_empty());
}

#[test]
fn install_dir_key_is_case_insensitive() {
    assert_eq!(parse_install_dir(ARMA3_MANIFEST).as_deref(), Some("Arma 3"));
    assert_eq!(parse_install_dir(r#""AppState" { "appid" "1" }"#), None);
    assert_eq!(parse_install_dir("not a manifest {"), None);
}

/// A fake home with the Steam root at `~/.local/share/Steam` whose `libraryfolders.vdf` lists
/// `<tmp>/lib2`.
struct FakeSteam {
    temp: tempfile::TempDir,
}

impl FakeSteam {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let fake = Self { temp };
        fs::create_dir_all(fake.root().join("steamapps")).unwrap();
        fs::create_dir_all(fake.library().join("steamapps")).unwrap();
        let folders = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            vdf_string(&fake.root()),
            vdf_string(&fake.library())
        );
        fs::write(fake.root().join("steamapps/libraryfolders.vdf"), folders).unwrap();
        fake
    }

    fn home(&self) -> PathBuf {
        self.temp.path().join("home")
    }

    fn root(&self) -> PathBuf {
        self.home().join(".local/share/Steam")
    }

    fn library(&self) -> PathBuf {
        self.temp.path().join("lib2")
    }

    fn env(&self) -> PlatformEnv {
        PlatformEnv::for_tests(Os::Linux, Some(self.home()), &[], None)
    }

    fn install_arma3(&self) -> PathBuf {
        let steamapps = self.library().join("steamapps");
        fs::write(steamapps.join("appmanifest_107410.acf"), ARMA3_MANIFEST).unwrap();
        let install_dir = steamapps.join("common/Arma 3");
        fs::create_dir_all(&install_dir).unwrap();
        install_dir
    }

    fn user_dir(&self) -> PathBuf {
        self.library()
            .join("steamapps/compatdata/107410/pfx/drive_c/users/steamuser")
    }
}

/// A path as a quoted VDF string body (backslashes escaped).
fn vdf_string(path: &Path) -> String {
    path.display().to_string().replace('\\', r"\\")
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn arma3_is_found_in_a_listed_library() {
    let fake = FakeSteam::new();
    let install_dir = fake.install_arma3();
    let registry = MemoryRegistry::default();
    assert_eq!(
        find_arma3(&fake.env(), &registry),
        Some(SteamApp {
            app_id: ARMA3_APP_ID,
            library: fake.library(),
            install_dir: install_dir.clone(),
        })
    );
    assert_eq!(detect_arma3_dir(&fake.env(), &registry), Some(install_dir));
}

#[test]
fn a_manifest_without_its_folder_is_not_an_install() {
    let fake = FakeSteam::new();
    let install_dir = fake.install_arma3();
    fs::remove_dir(install_dir).unwrap();
    assert_eq!(find_arma3(&fake.env(), &MemoryRegistry::default()), None);
}

#[cfg(unix)]
#[test]
fn symlinked_roots_are_listed_once() {
    let fake = FakeSteam::new();
    symlink(&fake.root(), &fake.home().join(".steam/root"));
    assert_eq!(
        steam_roots(&fake.env(), &MemoryRegistry::default()),
        vec![fake.root()]
    );
}

#[test]
fn the_flatpak_root_is_found() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().to_path_buf();
    let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    fs::create_dir_all(flatpak.join("steamapps/common/Arma 3")).unwrap();
    fs::write(
        flatpak.join("steamapps/appmanifest_107410.acf"),
        r#""AppState" { "appid" "107410" }"#,
    )
    .unwrap();
    let env = PlatformEnv::for_tests(Os::Linux, Some(home), &[], None);
    let registry = MemoryRegistry::default();
    assert_eq!(steam_roots(&env, &registry), vec![flatpak.clone()]);
    assert_eq!(
        find_arma3(&env, &registry).map(|app| app.install_dir),
        Some(flatpak.join("steamapps/common/Arma 3"))
    );
}

#[test]
fn proton_paths_follow_the_prefix_layout() {
    let fake = FakeSteam::new();
    fake.install_arma3();
    let registry = MemoryRegistry::default();
    let app = find_arma3(&fake.env(), &registry).unwrap();
    assert_eq!(proton_paths(&app), None);

    fs::create_dir_all(fake.user_dir().join("My Documents")).unwrap();
    let paths = proton_paths(&app).unwrap();
    assert!(paths.documents_arma3.ends_with("My Documents/Arma 3"));

    fs::create_dir_all(fake.user_dir().join("Documents")).unwrap();
    let paths = proton_paths(&app).unwrap();
    assert_eq!(
        paths.documents_arma3,
        fake.user_dir().join("Documents/Arma 3")
    );
    assert_eq!(
        arma3_documents_dir(&fake.env(), &MemoryFolders::default(), &registry),
        Some(fake.user_dir().join("Documents/Arma 3"))
    );

    assert_eq!(rpt_dir(&fake.env(), &registry), None);
    let local = fake.user_dir().join("AppData/Local/Arma 3");
    fs::create_dir_all(&local).unwrap();
    assert_eq!(paths.local_appdata_arma3, local);
    assert_eq!(rpt_dir(&fake.env(), &registry), Some(local));
}
