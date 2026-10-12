//! Registry queries, value decoding and the positive/negative cache.

use std::io;

use a3s_platform::registry::{
    CachedRegistry, Hive, MemoryRegistry, RegData, RegQuery, RegistryRead as _, RegistryWrite as _,
    decode_value, expand_env,
};

const ARMA3_KEY: &str = r"SOFTWARE\Wow6432Node\Bohemia Interactive\Arma 3";

fn no_vars(_: &str) -> Option<String> {
    None
}

#[test]
fn queries_use_backslashes_and_match_case_insensitively() {
    let query = RegQuery::new(
        Hive::LocalMachine,
        "SOFTWARE/Valve/Steam",
        Some("InstallPath"),
    );
    assert_eq!(query.key, r"SOFTWARE\Valve\Steam");

    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::LocalMachine,
        ARMA3_KEY,
        Some("MAIN"),
        RegData::Sz(r"D:\Arma 3".to_owned()),
    );
    let lowercase = RegQuery::new(Hive::LocalMachine, &ARMA3_KEY.to_lowercase(), Some("main"));
    assert_eq!(
        registry.read_string(&lowercase).unwrap().as_deref(),
        Some(r"D:\Arma 3")
    );
    let other_hive = RegQuery::new(Hive::CurrentUser, ARMA3_KEY, Some("MAIN"));
    assert_eq!(registry.read_string(&other_hive).unwrap(), None);

    registry.insert(
        Hive::CurrentUser,
        r"Software\TeamSpeak 3 Client",
        None,
        RegData::Sz(r"C:\TS3".to_owned()),
    );
    let default_value = RegQuery::new(Hive::CurrentUser, r"software\teamspeak 3 client", None);
    assert_eq!(
        registry.read_string(&default_value).unwrap().as_deref(),
        Some(r"C:\TS3")
    );
    let named = RegQuery::new(Hive::CurrentUser, r"Software\TeamSpeak 3 Client", Some("x"));
    assert_eq!(registry.read_string(&named).unwrap(), None);
}

#[test]
fn values_are_trimmed_like_java_and_blank_is_none() {
    assert_eq!(
        decode_value(&RegData::Sz("  C:\\Arma 3 \r\n".to_owned()), &no_vars).as_deref(),
        Some(r"C:\Arma 3")
    );
    assert_eq!(decode_value(&RegData::Sz(String::new()), &no_vars), None);
    assert_eq!(decode_value(&RegData::Sz("   ".to_owned()), &no_vars), None);
    assert_eq!(decode_value(&RegData::Dword(1), &no_vars), None);
}

#[test]
fn expand_sz_values_are_expanded() {
    let vars = |name: &str| (name == "ProgramFiles").then(|| r"C:\Program Files".to_owned());
    assert_eq!(
        decode_value(
            &RegData::ExpandSz(r"%ProgramFiles%\Steam".to_owned()),
            &vars
        )
        .as_deref(),
        Some(r"C:\Program Files\Steam")
    );
    assert_eq!(expand_env(r"%FOO%\x", &vars), r"%FOO%\x");
    assert_eq!(expand_env("100%", &vars), "100%");
    assert_eq!(expand_env("%%ProgramFiles%", &vars), r"%C:\Program Files");

    let registry = MemoryRegistry::with_env(&[("programfiles", r"C:\Program Files")]);
    registry.insert(
        Hive::LocalMachine,
        "k",
        Some("v"),
        RegData::ExpandSz(r"%ProgramFiles%\Steam".to_owned()),
    );
    let query = RegQuery::new(Hive::LocalMachine, "k", Some("v"));
    assert_eq!(
        registry.read_string(&query).unwrap().as_deref(),
        Some(r"C:\Program Files\Steam")
    );
}

#[test]
fn cache_keeps_found_and_missing_values() {
    let registry = CachedRegistry::new(MemoryRegistry::default());
    registry.inner().insert(
        Hive::LocalMachine,
        ARMA3_KEY,
        Some("MAIN"),
        RegData::Sz(r"D:\Arma 3".to_owned()),
    );
    let found = RegQuery::new(Hive::LocalMachine, ARMA3_KEY, Some("MAIN"));
    assert_eq!(
        registry.read_string(&found).unwrap().as_deref(),
        Some(r"D:\Arma 3")
    );
    assert_eq!(registry.inner().query_count(), 1);
    let same_in_other_case =
        RegQuery::new(Hive::LocalMachine, &ARMA3_KEY.to_uppercase(), Some("main"));
    assert_eq!(
        registry
            .read_string(&same_in_other_case)
            .unwrap()
            .as_deref(),
        Some(r"D:\Arma 3")
    );
    assert_eq!(registry.inner().query_count(), 1);

    let missing = RegQuery::new(Hive::CurrentUser, "Software\\Nothing", None);
    assert_eq!(registry.read_string(&missing).unwrap(), None);
    assert_eq!(registry.read_string(&missing).unwrap(), None);
    assert_eq!(registry.inner().query_count(), 2);

    // A value that appears later stays missing: Java never invalidated its cache.
    registry.inner().insert(
        Hive::CurrentUser,
        "Software\\Nothing",
        None,
        RegData::Sz("now".to_owned()),
    );
    assert_eq!(registry.read_string(&missing).unwrap(), None);
    assert_eq!(registry.inner().query_count(), 2);

    registry.clear();
    assert_eq!(
        registry.read_string(&missing).unwrap().as_deref(),
        Some("now")
    );
    assert_eq!(registry.inner().query_count(), 3);
}

#[test]
fn cache_turns_errors_into_cached_misses() {
    let registry = CachedRegistry::new(MemoryRegistry::default());
    let denied = RegQuery::new(Hive::LocalMachine, "SOFTWARE\\Locked", Some("v"));
    registry
        .inner()
        .fail_with(&denied, io::ErrorKind::PermissionDenied);
    assert!(registry.inner().read_string(&denied).is_err());
    assert_eq!(registry.inner().query_count(), 1);

    assert_eq!(registry.read_string(&denied).unwrap(), None);
    assert_eq!(registry.read_string(&denied).unwrap(), None);
    assert_eq!(registry.inner().query_count(), 2);
}

#[test]
fn writes_through_the_cache_refresh_the_value() {
    let registry = CachedRegistry::new(MemoryRegistry::default());
    let query = RegQuery::new(Hive::CurrentUser, "Software\\Run", Some("app"));
    assert_eq!(registry.read_string(&query).unwrap(), None);

    registry
        .set_string(Hive::CurrentUser, "Software\\Run", "app", "cmd")
        .unwrap();
    assert_eq!(
        registry.read_string(&query).unwrap().as_deref(),
        Some("cmd")
    );

    registry
        .delete_value(Hive::CurrentUser, "software\\run", "APP")
        .unwrap();
    assert_eq!(registry.read_string(&query).unwrap(), None);
}
