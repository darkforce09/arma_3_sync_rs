//! Port of `src/test/java/fr/soe/a3s/dao/PreferencesDAOTest.java` (A16).

use a3s_paths::AppDirs;
use a3s_repo_files::local::LocalStore;
use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, StartWithOs,
};
use a3s_types::local::Preferences;

#[test]
fn a16_write_then_read_preferences_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = AppDirs::from_roots(
        tmp.path().join("config"),
        tmp.path().join("data"),
        tmp.path().join("cache"),
    );
    let preferences = Preferences {
        launch_panel_game_launch: MinimizationType::Tray,
        look_and_feel: LookAndFeel::LafHifi,
        icon_resize_size: IconResize::Auto,
        start_with_os: StartWithOs::Enabled,
        check_repositories_frequency: CheckRepositoriesFrequency::Freq2,
        ..Preferences::default()
    };

    LocalStore::new(&dirs)
        .write_preferences(&preferences)
        .unwrap();
    let file = tmp.path().join("config").join("preferences.toml");
    assert!(
        file.is_file(),
        "Preferences file should be created under the user config directory"
    );

    let restored = LocalStore::new(&dirs).read_preferences().unwrap();
    assert_eq!(restored.look_and_feel, LookAndFeel::LafHifi);
    assert_eq!(restored.start_with_os, StartWithOs::Enabled);
    assert_eq!(
        restored.check_repositories_frequency,
        CheckRepositoriesFrequency::Freq2
    );
    assert_eq!(restored.launch_panel_game_launch, MinimizationType::Tray);
    assert_eq!(restored, preferences);
}
