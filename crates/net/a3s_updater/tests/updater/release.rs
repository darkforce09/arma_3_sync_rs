//! Parsing of the JSON manifest, the GitHub API response and the legacy XML.

use a3s_updater::{
    ReleaseFormat, UpdateError, parse_github_release, parse_json_manifest, parse_legacy_xml,
    read_local_xml,
};
use url::Url;

use crate::support::HASH;

const TARGET: &str = "x86_64-unknown-linux-gnu";

fn source() -> Url {
    Url::parse("https://updates.example/channel/a3s.json").expect("url")
}

fn manifest(json: &str) -> Result<a3s_updater::Release, UpdateError> {
    parse_json_manifest(json, &source(), TARGET)
}

#[test]
fn strict_manifest_with_integer_size() {
    let release = manifest(&format!(
        r#"{{"version":"2026.2.1","file":"Arma3Sync-2026.2.1.zip","sha256":"{HASH}","size":1234}}"#
    ))
    .expect("manifest");
    assert_eq!(release.version, "2026.2.1");
    assert_eq!(release.size, 1234);
    assert_eq!(release.file_name, "Arma3Sync-2026.2.1.zip");
    assert_eq!(
        release.download_url.as_str(),
        "https://updates.example/channel/Arma3Sync-2026.2.1.zip"
    );
    assert_eq!(release.sha256.as_deref(), Some(HASH));
    assert_eq!(release.format, ReleaseFormat::Json);
    assert_eq!(release.source, source().as_str());
}

#[test]
fn manifest_aliases_derivation_and_normalization() {
    let upper = HASH.to_ascii_uppercase();
    let release = manifest(&format!(
        r#"{{"schemaVersion":7,"nom":" 2026.2.1 ","url":"../files/Arma3Sync%20new.zip","hash":"{upper}","size":1e3}}"#
    ))
    .expect("aliases");
    assert_eq!(release.version, "2026.2.1");
    assert_eq!(release.file_name, "Arma3Sync new.zip");
    assert_eq!(
        release.download_url.as_str(),
        "https://updates.example/files/Arma3Sync%20new.zip"
    );
    assert_eq!(release.sha256.as_deref(), Some(HASH));
    assert_eq!(release.size, 1000);

    let release = manifest(&format!(
        r#"{{"version":"2026.2.1","filename":"a.zip","downloadUrl":"https://cdn.example/b.zip","sha256":"{HASH}","size":1.0}}"#
    ))
    .expect("explicit download");
    assert_eq!(release.file_name, "a.zip");
    assert_eq!(release.download_url.as_str(), "https://cdn.example/b.zip");
    assert_eq!(release.size, 1);
}

#[test]
fn manifest_rejections() {
    let invalid = [
        format!(r#"{{"version":"2026.2.1","file":"update.zip","sha256":"{HASH}","size":1.5}}"#),
        format!(r#"{{"version":"2026.2","file":"update.zip","sha256":"{HASH}"}}"#),
        format!(r#"{{"version":"2026.2.1","file":"update.zip","sha256":"{HASH}","size":01}}"#),
        format!(r#"{{"version":"2026.2.1","file":"update.zip","sha256":"{HASH}","size":-1}}"#),
        format!(r#"{{"version":"2026.2.1","file":"update.zip","sha256":"{HASH}","size":1e30}}"#),
        format!(r#"{{"version":"2026.2.1","file":"update.zip","sha256":"{HASH}","size":"1"}}"#),
        format!(
            r#"{{"version":"2026.2.1","file":"update.zip","downloadUrl":"file:///tmp/update.zip","sha256":"{HASH}"}}"#
        ),
        format!(
            r#"{{"version":"2026.2.1","file":"update.zip","downloadUrl":"https://user:pw@cdn.example/u.zip","sha256":"{HASH}"}}"#
        ),
        format!(r#"{{"version":"2026.2.1","file":"../update.zip","sha256":"{HASH}"}}"#),
        format!(r#"{{"version":"2026.2.1","file":"update.tar.gz","sha256":"{HASH}"}}"#),
        format!(r#"{{"version":"2026.2.1","sha256":"{HASH}"}}"#),
        format!(r#"{{"version":2026,"file":"update.zip","sha256":"{HASH}"}}"#),
        r#"{"version":"2026.2.1","file":"update.zip","sha256":"abc"}"#.to_owned(),
        format!(
            r#"{{"version":"2026.2.1","version":"2026.2.2","file":"u.zip","sha256":"{HASH}"}}"#
        ),
        format!(r#"{{"version":"2026.2.1","file":"u.zip","sha256":"{HASH}","x":{{"a":1,"a":2}}}}"#),
        format!(r#"[{{"version":"2026.2.1","file":"u.zip","sha256":"{HASH}"}}]"#),
        format!(r#"{{"version":"2026.2.1","file":"u.zip","sha256":"{HASH}"}} trailing"#),
    ];
    for json in invalid {
        assert!(manifest(&json).is_err(), "accepted {json}");
    }
}

#[test]
fn manifest_target_table() {
    let other = "1".repeat(64);
    let json = format!(
        r#"{{"version":"2026.3.0","file":"generic.zip","sha256":"{HASH}",
            "targets":{{
              "{TARGET}":{{"file":"linux.zip","sha256":"{other}","size":10}},
              "x86_64-pc-windows-msvc":{{"downloadUrl":"https://cdn.example/win.zip","hash":"{HASH}"}}
            }}}}"#
    );
    let linux = parse_json_manifest(&json, &source(), TARGET).expect("linux");
    assert_eq!(linux.file_name, "linux.zip");
    assert_eq!(linux.sha256.as_deref(), Some(other.as_str()));
    assert_eq!(linux.size, 10);
    assert_eq!(linux.version, "2026.3.0");

    let windows = parse_json_manifest(&json, &source(), "x86_64-pc-windows-msvc").expect("win");
    assert_eq!(windows.file_name, "win.zip");
    assert_eq!(windows.download_url.as_str(), "https://cdn.example/win.zip");

    let missing = parse_json_manifest(&json, &source(), "aarch64-apple-darwin");
    assert!(matches!(missing, Err(UpdateError::NoTarget(t)) if t == "aarch64-apple-darwin"));

    let single = format!(r#"{{"version":"2026.3.0","file":"generic.zip","sha256":"{HASH}"}}"#);
    let release = parse_json_manifest(&single, &source(), "aarch64-apple-darwin").expect("single");
    assert_eq!(release.file_name, "generic.zip");
}

fn github(json: &str, pattern: &str) -> Result<a3s_updater::Release, UpdateError> {
    let api = Url::parse("https://api.github.com/repos/o/r/releases/latest").expect("url");
    parse_github_release(json, &api, pattern, TARGET)
}

fn github_release(tag: &str, assets: &str) -> String {
    format!(r#"{{"tag_name":"{tag}","draft":true,"assets":[{assets}]}}"#)
}

fn asset(name: &str, url: &str, digest: &str) -> String {
    format!(r#"{{"name":"{name}","browser_download_url":"{url}","digest":"{digest}","size":1234}}"#)
}

#[test]
fn github_asset_selection_and_tag_normalization() {
    let assets = format!(
        r#"5, {}, {}, {}"#,
        asset(
            "Arma3Sync-2026.1.1.zip ",
            "https://dl.example/wrong.zip",
            HASH
        ),
        asset(
            "Arma3Sync-2026.1.1.zip",
            "https://dl.example/right.zip",
            &format!("SHA256:{}", HASH.to_ascii_uppercase())
        ),
        asset(
            "Arma3Sync-2026.1.1.zip",
            "https://dl.example/second.zip",
            HASH
        ),
    );
    let release = github(
        &github_release("vV2026.1.1", &assets),
        "Arma3Sync-{version}.zip",
    )
    .expect("release");
    assert_eq!(release.version, "2026.1.1");
    assert_eq!(release.tag.as_deref(), Some("vV2026.1.1"));
    assert_eq!(release.file_name, "Arma3Sync-2026.1.1.zip");
    assert_eq!(
        release.download_url.as_str(),
        "https://dl.example/right.zip"
    );
    assert_eq!(release.sha256.as_deref(), Some(HASH));
    assert_eq!(release.size, 1234);
    assert_eq!(release.format, ReleaseFormat::GitHub);

    let tagged = github(
        &github_release(
            "v2026.1.1",
            &asset("A-v2026.1.1.zip", "https://dl.example/a.zip", HASH),
        ),
        "A-{tag}.zip",
    )
    .expect("tag placeholder");
    assert_eq!(tagged.file_name, "A-v2026.1.1.zip");

    let name = format!("A-2026.1.1-{TARGET}.zip");
    let targeted = github(
        &github_release("2026.1.1", &asset(&name, "http://dl.example/a.zip", HASH)),
        "A-{version}-{target}.zip",
    )
    .expect("target placeholder, http accepted by the parser");
    assert_eq!(targeted.file_name, name);
}

#[test]
fn github_rejections() {
    let ok_asset = asset("A-2026.1.1.zip", "https://dl.example/a.zip", HASH);
    let cases = [
        (github_release("2026.1.1", &ok_asset), "../A-{version}.zip"),
        (github_release("2026.1.1", &ok_asset), "A-{version}.tar.gz"),
        (github_release("2026.1", &ok_asset), "A-{version}.zip"),
        (github_release(" ", &ok_asset), "A-{version}.zip"),
        (github_release("2026.1.1", &ok_asset), "B-{version}.zip"),
        (
            github_release(
                "2026.1.1",
                &asset("A-2026.1.1.zip", "https://dl.example/a.zip", "abc"),
            ),
            "A-{version}.zip",
        ),
        (
            github_release(
                "2026.1.1",
                r#"{"name":"A-2026.1.1.zip","browser_download_url":"https://dl.example/a.zip"}"#,
            ),
            "A-{version}.zip",
        ),
        (
            github_release(
                "2026.1.1",
                &asset("A-2026.1.1.zip", "ftp://dl.example/a.zip", HASH),
            ),
            "A-{version}.zip",
        ),
        (
            github_release(
                "2026.1.1",
                &asset("A-2026.1.1.zip", "https://u:p@dl.example/a.zip", HASH),
            ),
            "A-{version}.zip",
        ),
        (
            r#"{"tag_name":"2026.1.1","assets":{}}"#.to_owned(),
            "A-{version}.zip",
        ),
    ];
    for (json, pattern) in cases {
        assert!(
            github(&json, pattern).is_err(),
            "accepted {json} with {pattern}"
        );
    }
    // A digest without the `sha256:` prefix is accepted.
    assert!(github(&github_release("2026.1.1", &ok_asset), "A-{version}.zip").is_ok());
}

fn xml(text: &str) -> Result<Option<a3s_updater::Release>, UpdateError> {
    let source = Url::parse("https://updates.example/legacy/a3s.xml").expect("url");
    parse_legacy_xml(text, &source, "Arma3Sync-{version}-{target}.zip", TARGET)
}

#[test]
fn legacy_xml() {
    let release = xml("<root><info><nom> 1.6.0 </nom></info><file>Old-1.6.0.zip</file></root>")
        .expect("parse")
        .expect("release");
    assert_eq!(release.version, "1.6.0");
    assert_eq!(release.file_name, "Old-1.6.0.zip");
    assert_eq!(
        release.download_url.as_str(),
        "https://updates.example/legacy/Old-1.6.0.zip"
    );
    assert_eq!(release.sha256, None);
    assert_eq!(release.size, 0);
    assert_eq!(release.format, ReleaseFormat::Xml);

    let derived = xml("<a3s><nom>2026.<b>1</b>.1</nom></a3s>")
        .expect("parse")
        .expect("release");
    assert_eq!(derived.version, "2026.1.1");
    assert_eq!(
        derived.file_name,
        format!("Arma3Sync-2026.1.1-{TARGET}.zip")
    );

    assert_eq!(
        xml("<a3s><nom>  </nom><file>x.zip</file></a3s>").expect("blank"),
        None
    );
    assert_eq!(xml("<a3s><file>x.zip</file></a3s>").expect("no nom"), None);
    assert!(xml("<!DOCTYPE a3s [<!ENTITY x \"1\">]><a3s><nom>&x;</nom></a3s>").is_err());
    assert!(xml("<a3s><nom>1.0.0</nom><file>../x.zip</file></a3s>").is_err());
    assert!(xml("<a3s><nom>1.0.0</nom><file>x.exe</file></a3s>").is_err());
    assert!(xml("<a3s><nom>1.0.0</nom>").is_err());
}

#[test]
fn local_legacy_xml() {
    let install = tempfile::tempdir().expect("tempdir");
    let pattern = "Arma3Sync-{version}.zip";
    assert_eq!(
        read_local_xml(install.path(), pattern, TARGET).expect("absent"),
        None
    );
    std::fs::create_dir(install.path().join("a3s.xml")).expect("dir");
    assert_eq!(
        read_local_xml(install.path(), pattern, TARGET).expect("not a file"),
        None
    );
    std::fs::remove_dir(install.path().join("a3s.xml")).expect("rmdir");

    std::fs::write(
        install.path().join("a3s.xml"),
        "<a3s><nom>1.5.0</nom></a3s>",
    )
    .expect("write");
    let release = read_local_xml(install.path(), pattern, TARGET)
        .expect("read")
        .expect("release");
    assert_eq!(release.file_name, "Arma3Sync-1.5.0.zip");
    assert!(release.source.starts_with("file://"));
}
