//! `updater.toml` loading, defaults, dev fallbacks and overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use a3s_updater::{ConfigOverrides, DEFAULT_ASSET_PATTERN, UpdateConfig, UpdateError};

fn parse(toml: &str, overrides: &ConfigOverrides) -> UpdateConfig {
    UpdateConfig::from_toml_str(toml, overrides).expect("config")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

#[test]
fn defaults_without_a_file() {
    let install = tempfile::tempdir().expect("tempdir");
    let config = UpdateConfig::load(install.path(), &ConfigOverrides::default()).expect("load");
    for dev in [false, true] {
        assert_eq!(
            config.manifest_url(dev),
            "https://arma3sync.vpzbrig21.de/updates/a3s.json"
        );
        assert_eq!(
            config.legacy_xml_url(dev),
            "https://arma3sync.vpzbrig21.de/updates/a3s.xml"
        );
        assert_eq!(
            config.github_api_url(dev),
            "https://api.github.com/repos/vpzbrig21/Arma3Sync/releases/latest"
        );
        assert_eq!(config.github_asset_pattern(dev), DEFAULT_ASSET_PATTERN);
    }
    assert!(config.github_enabled());
    assert!(!config.allow_http());
    assert_eq!(config.connect_timeout(), Duration::from_secs(30));
    assert_eq!(config.read_timeout(), Duration::from_secs(30));
}

#[test]
fn exactly_one_file_is_read_in_precedence_order() {
    let root = tempfile::tempdir().expect("tempdir");
    let install = root.path().join("installation");
    let user = root.path().join("user/configuration");
    let installed = install.join(UpdateConfig::RELATIVE_PATH);
    write(
        &installed,
        "[update]\nmanifest_url = \"https://installed.example/a3s.json\"\nallow_http = true\n",
    );
    let user_overrides = ConfigOverrides {
        user_config_dir: Some(user.clone()),
        ..ConfigOverrides::default()
    };

    let config = UpdateConfig::load(&install, &user_overrides).expect("installed");
    assert_eq!(
        config.manifest_url(false),
        "https://installed.example/a3s.json"
    );
    assert!(config.allow_http());

    write(
        &install.join("updater.toml"),
        "[update]\nmanifest_url = \"https://root.example/a3s.json\"\n",
    );
    let config = UpdateConfig::load(&install, &user_overrides).expect("root");
    assert_eq!(config.manifest_url(false), "https://root.example/a3s.json");
    // A key present only in the lower-priority file is not merged in.
    assert!(!config.allow_http());

    write(
        &user.join("updater.toml"),
        "[update]\nmanifest_url = \"https://user.example/a3s.json\"\n",
    );
    let config = UpdateConfig::load(&install, &user_overrides).expect("user");
    assert_eq!(config.manifest_url(false), "https://user.example/a3s.json");

    // Without the override the user file is not consulted.
    let config = UpdateConfig::load(&install, &ConfigOverrides::default()).expect("no user");
    assert_eq!(config.manifest_url(false), "https://root.example/a3s.json");
}

#[test]
fn dev_values_fall_back_to_file_values_before_overrides() {
    let file = "[update]\nmanifest_url = \"https://file.example/a3s.json\"\n\
                legacy_xml_url = \"https://file.example/a3s.xml\"\n\
                dev_legacy_xml_url = \"  \"\n\
                [github]\napi_url = \"https://api.example/latest\"\n\
                asset_pattern = \"A-{version}.zip\"\n";
    let overrides = ConfigOverrides {
        manifest_url: Some("https://override.example/a3s.json".to_owned()),
        ..ConfigOverrides::default()
    };
    let config = parse(file, &overrides);
    assert_eq!(
        config.manifest_url(false),
        "https://override.example/a3s.json"
    );
    assert_eq!(config.manifest_url(true), "https://file.example/a3s.json");
    // A blank dev value falls back too.
    assert_eq!(config.legacy_xml_url(true), "https://file.example/a3s.xml");
    assert_eq!(config.github_api_url(true), "https://api.example/latest");
    assert_eq!(config.github_asset_pattern(true), "A-{version}.zip");

    let base = ConfigOverrides {
        base_url: Some("https://base.example/updates/".to_owned()),
        ..ConfigOverrides::default()
    };
    let config = parse("", &base);
    assert_eq!(
        config.manifest_url(false),
        "https://base.example/updates/a3s.json"
    );
    assert_eq!(
        config.legacy_xml_url(true),
        "https://base.example/updates/a3s.xml"
    );

    let blank_base = ConfigOverrides {
        base_url: Some(" ".to_owned()),
        ..ConfigOverrides::default()
    };
    assert_eq!(
        parse("", &blank_base).manifest_url(false),
        "https://arma3sync.vpzbrig21.de/updates/a3s.json"
    );

    // A blank manifest URL stays blank (it disables that source); its dev value falls back to it.
    let config = parse(
        "[update]\nmanifest_url = \"\"\n",
        &ConfigOverrides::default(),
    );
    assert_eq!(config.manifest_url(false), "");
    assert_eq!(config.manifest_url(true), "");

    let empty_override = ConfigOverrides {
        legacy_xml_url: Some(String::new()),
        github_dev_api_url: Some("https://dev.example/latest".to_owned()),
        ..ConfigOverrides::default()
    };
    let config = parse("", &empty_override);
    assert_eq!(config.legacy_xml_url(false), "");
    assert_eq!(config.github_api_url(true), "https://dev.example/latest");
}

#[test]
fn boolean_overrides_only_force_true() {
    let forced = ConfigOverrides {
        allow_http: true,
        github_enabled: true,
        ..ConfigOverrides::default()
    };
    let file = "[update]\nallow_http = false\n[github]\nenabled = false\n";
    let config = parse(file, &forced);
    assert!(config.allow_http());
    assert!(config.github_enabled());

    let config = parse(file, &ConfigOverrides::default());
    assert!(!config.allow_http());
    assert!(!config.github_enabled());

    let config = parse("[update]\nallow_http = true\n", &ConfigOverrides::default());
    assert!(config.allow_http());
}

#[test]
fn wrong_value_types_and_invalid_toml_are_errors() {
    let invalid = [
        "[update]\nallow_http = \"true\"\n",
        "[update]\nmanifest_url = 5\n",
        "[update]\nconnect_timeout_ms = 0\n",
        "[update]\nread_timeout_ms = 300001\n",
        "[update]\nread_timeout_ms = \"5\"\n",
        "[update\nmanifest_url = \"x\"\n",
        "[update]\nmanifest_url = https://x\n",
    ];
    for text in invalid {
        let result = UpdateConfig::from_toml_str(text, &ConfigOverrides::default());
        assert!(
            matches!(result, Err(UpdateError::Config { .. })),
            "{text:?} gave {result:?}"
        );
    }

    let config = parse(
        "[update]\nconnect_timeout_ms = 1\nread_timeout_ms = 300000\nunknown = [1, 2]\n\
         [other]\nkey = 1\n",
        &ConfigOverrides::default(),
    );
    assert_eq!(config.connect_timeout(), Duration::from_millis(1));
    assert_eq!(config.read_timeout(), Duration::from_millis(300_000));
}

#[test]
fn file_errors_name_the_file() {
    let install = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        install.path().join("updater.toml"),
        "[github]\nenabled = 1\n",
    )
    .expect("write");
    match UpdateConfig::load(install.path(), &ConfigOverrides::default()) {
        Err(UpdateError::Config { path, .. }) => {
            assert_eq!(path, Some(install.path().join("updater.toml")));
        }
        other => panic!("expected a config error, got {other:?}"),
    }
}

#[test]
fn overrides_from_environment_variables() {
    let vars: HashMap<&str, &str> = [
        ("A3S_UPDATER_USER_CONFIG_PATH", "/home/user/config"),
        ("A3S_UPDATER_URL", "https://base.example"),
        ("A3S_UPDATER_MANIFEST_URL", "https://m.example/a3s.json"),
        ("A3S_UPDATER_LEGACY_XML_URL", ""),
        ("A3S_UPDATER_MANIFEST_URL_DEV", "https://m.example/dev.json"),
        (
            "A3S_UPDATER_LEGACY_XML_URL_DEV",
            "https://m.example/dev.xml",
        ),
        ("A3S_UPDATER_ALLOW_HTTP", "TRUE"),
        ("A3S_UPDATER_GITHUB_ENABLED", "yes"),
        ("A3S_UPDATER_GITHUB_API_URL", "https://api.example/latest"),
        ("A3S_UPDATER_GITHUB_API_URL_DEV", "https://api.example/dev"),
    ]
    .into_iter()
    .collect();
    let overrides = ConfigOverrides::from_env_with(|name| vars.get(name).map(|v| (*v).to_owned()));
    assert_eq!(
        overrides,
        ConfigOverrides {
            user_config_dir: Some(PathBuf::from("/home/user/config")),
            base_url: Some("https://base.example".to_owned()),
            manifest_url: Some("https://m.example/a3s.json".to_owned()),
            legacy_xml_url: Some(String::new()),
            dev_manifest_url: Some("https://m.example/dev.json".to_owned()),
            dev_legacy_xml_url: Some("https://m.example/dev.xml".to_owned()),
            github_api_url: Some("https://api.example/latest".to_owned()),
            github_dev_api_url: Some("https://api.example/dev".to_owned()),
            allow_http: true,
            github_enabled: false,
        }
    );

    let blank = ConfigOverrides::from_env_with(|name| {
        (name == "A3S_UPDATER_USER_CONFIG_PATH").then(|| "  ".to_owned())
    });
    assert_eq!(blank, ConfigOverrides::default());
}

#[test]
fn optional_github_settings_are_read_verbatim() {
    let install = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        install.path().join("updater.toml"),
        "[github]\nenabled = true\n\
         api_url = \"https://api.github.com/repos/example/project/releases/latest\"\n\
         asset_pattern = \"Arma3Sync-{tag}.zip\"\n",
    )
    .expect("write");
    let config = UpdateConfig::load(install.path(), &ConfigOverrides::default()).expect("load");
    assert!(config.github_enabled());
    assert_eq!(
        config.github_api_url(false),
        "https://api.github.com/repos/example/project/releases/latest"
    );
    assert_eq!(config.github_asset_pattern(false), "Arma3Sync-{tag}.zip");
}

#[test]
fn github_is_enabled_unless_the_file_disables_it() {
    let install = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        install.path().join("updater.toml"),
        "[update]\nmanifest_url = \"https://updates.example/a3s.json\"\n",
    )
    .expect("write");
    let config = UpdateConfig::load(install.path(), &ConfigOverrides::default()).expect("load");
    assert!(config.github_enabled());
}
