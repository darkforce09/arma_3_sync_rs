//! Preset import: HTML parsing, text cleaning and comparison with local addons.

use std::fs;
use std::path::Path;

use a3s_tools::preset::{
    DEFAULT_IMPORT_PRESET_NAME, ImportError, ImportResult, compare, import_preset,
};
use a3s_types::constants::GameDlc;
use a3s_types::local::Addon;

const WORKSHOP: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=";

fn preset(name: &str, rows: &[String]) -> String {
    format!(
        "<html><head><meta name=\"arma:PresetName\" content=\"{name}\" /></head><body>{}</body></html>",
        rows.concat()
    )
}

fn row(name: &str, url: &str) -> String {
    format!(
        "<tr data-type=\"ModContainer\"><td data-type=\"DisplayName\">{name}</td><td><a href=\"{url}\" data-type=\"Link\">{url}</a></td></tr>"
    )
}

fn dlc_row(name: &str, url: &str) -> String {
    format!(
        "<tr data-type=\"DlcContainer\"><td data-type=\"DisplayName\">{name}</td><td><a href=\"{url}\" data-type=\"Link\">{url}</a></td></tr>"
    )
}

fn compare_alone(html: &str) -> ImportResult {
    compare(html, &[]).unwrap()
}

/// The display name the importer reads from one Workshop row named `name`.
fn display_name(name: &str) -> String {
    compare_alone(&preset("p", &[row(name, &format!("{WORKSHOP}1"))])).missing[0]
        .name
        .clone()
}

/// The preset name the importer reads from `content`.
fn preset_name(content: &str) -> String {
    compare_alone(&preset(content, &[row("m", &format!("{WORKSHOP}1"))])).preset_name
}

fn addon_dir(parent: &Path, folder: &str, meta: Option<&str>) {
    let dir = parent.join(folder);
    fs::create_dir_all(&dir).unwrap();
    if let Some(meta) = meta {
        fs::write(dir.join("meta.cpp"), meta).unwrap();
    }
}

// ----- Java tests -----

#[test]
fn reports_missing_workshop_mods_by_id_and_link() {
    let tmp = tempfile::tempdir().unwrap();
    addon_dir(
        tmp.path(),
        "Local Mod",
        Some("publishedid = 1001;\nname = \"Local Display Name\";\n"),
    );
    let html = preset(
        "Test preset",
        &[
            row("Local Display Name", &format!("{WORKSHOP}1001")),
            row("Missing Display Name", &format!("{WORKSHOP}2002")),
        ],
    );
    let addons = [Addon::new(
        "local",
        "Local Mod",
        tmp.path().to_str().unwrap(),
    )];

    let result = compare(&html, &addons).unwrap();

    assert_eq!(result.preset_name, "Test preset");
    assert_eq!(result.requested_count, 2);
    assert_eq!(result.matched.len(), 1);
    assert_eq!(
        result.matched[0].local_name.as_deref(),
        Some("Local Display Name")
    );
    assert_eq!(result.matched[0].local_addon_key.as_deref(), Some("local"));
    assert_eq!(result.missing.len(), 1);
    assert_eq!(result.missing[0].published_id, "2002");
    assert_eq!(result.missing[0].name, "Missing Display Name");
    assert_eq!(result.missing[0].url, format!("{WORKSHOP}2002"));
}

#[test]
fn ignores_dlc_rows_and_duplicate_workshop_ids() {
    let html = format!(
        "<html><head></head><body>{}{}<tr data-type=\"DlcContainer\"><td data-type=\"DisplayName\">DLC</td><td><a data-type=\"Link\" href=\"https://store.steampowered.com/app/123\">Link</a></td></tr></body></html>",
        row("Same Mod", &format!("{WORKSHOP}3003")),
        row("Duplicate Mod", &format!("{WORKSHOP}3003")),
    );

    let result = compare_alone(&html);

    assert_eq!(result.requested_count, 1);
    assert_eq!(result.missing.len(), 1);
    assert_eq!(result.missing[0].published_id, "3003");
    assert_eq!(result.missing[0].name, "Same Mod");
    // The unknown CDLC row is kept as a missing CDLC.
    assert_eq!(result.requested_dlc_count, 1);
    let unknown = &result.missing_dlc[0];
    assert_eq!(unknown.steam_app_id, "123");
    assert_eq!(unknown.name, "DLC");
    assert_eq!(unknown.addon_key, None);
    assert_eq!(unknown.dlc, None);
}

#[test]
fn imports_installed_and_missing_cdlcs() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("GM")).unwrap();
    let html = preset(
        "CDLC preset",
        &[
            dlc_row(
                "Global Mobilization",
                "https://store.steampowered.com/app/1042220",
            ),
            dlc_row(
                "Western Sahara",
                "https://store.steampowered.com/app/1681170",
            ),
        ],
    );
    let addons = [Addon::new("dlc-gm", "GM", tmp.path().to_str().unwrap())];

    let result = compare(&html, &addons).unwrap();

    assert_eq!(result.requested_count, 0);
    assert_eq!(result.requested_dlc_count, 2);
    assert_eq!(result.matched_dlc.len(), 1);
    assert_eq!(result.matched_dlc[0].local_addon_key.as_deref(), Some("GM"));
    assert_eq!(result.missing_dlc.len(), 1);
    assert_eq!(result.missing_dlc[0].addon_key.as_deref(), Some("WS"));
    assert_eq!(result.missing_dlc[0].steam_app_id, "1681170");
}

#[test]
fn accepts_preset_containing_only_cdlcs() {
    let html = preset(
        "Only CDLC",
        &[dlc_row(
            "Contact",
            "https://store.steampowered.com/app/1021790",
        )],
    );

    let result = compare_alone(&html);

    assert_eq!(result.requested_count, 0);
    assert_eq!(result.requested_dlc_count, 1);
    assert_eq!(result.missing_dlc.len(), 1);
}

#[test]
fn rejects_html_without_workshop_mod_entries() {
    assert!(matches!(
        compare("<html><body>No mods</body></html>", &[]),
        Err(ImportError::NoEntries)
    ));
    assert!(matches!(compare("  \n", &[]), Err(ImportError::Empty)));
    assert!(matches!(compare("", &[]), Err(ImportError::Empty)));
}

// ----- Text cleaning -----

#[test]
fn display_names_are_cleaned() {
    assert_eq!(display_name("<b>A<br/>&amp;B"), "A&B");
    assert_eq!(display_name("A&amp;B"), "A&B");
    assert_eq!(display_name("A&AMP;B"), "A&B");
    assert_eq!(display_name("&#x41;&#X41;&#65;"), "AAA");
    assert_eq!(display_name("&amp;lt;"), "&lt;");
    assert_eq!(display_name("&lt;b&gt;"), "<b>");
    assert_eq!(display_name("&quot;q&quot; &apos;a&apos;"), "\"q\" 'a'");
    assert_eq!(display_name(" a \t\n b "), "a b");
    assert_eq!(display_name("a&#10;b"), "a b");
    assert_eq!(display_name("a\u{A0}b"), "a\u{A0}b");
    assert_eq!(display_name("\u{1}a"), "a");
}

#[test]
fn numeric_entities_decode_to_code_points() {
    assert_eq!(display_name("&#x1F600;"), "\u{1F600}");
    assert_eq!(display_name("&#xD83D;&#xDE00;"), "\u{1F600}");
    assert_eq!(display_name("&#55357;&#56832;"), "\u{1F600}");
    assert_eq!(display_name("x&#xD83D;y"), "x\u{FFFD}y");
    assert_eq!(display_name("x&#xDE00;y"), "x\u{FFFD}y");
    assert_eq!(display_name("&#abc;"), "&#abc;");
    assert_eq!(display_name("&#x110000;"), "&#x110000;");
    assert_eq!(display_name("&#99999999999;"), "&#99999999999;");
}

#[test]
fn display_name_stops_at_the_first_closing_tag() {
    let html = preset(
        "p",
        &["<tr data-type=\"ModContainer\"><td data-type=\"DisplayName\"><b>Foo</b> bar</td><td><a href=\"https://steamcommunity.com/sharedfiles/filedetails/?id=5\" data-type=\"Link\">x</a></td></tr>".to_owned()],
    );
    assert_eq!(compare_alone(&html).missing[0].name, "Foo");
}

#[test]
fn preset_names() {
    assert_eq!(preset_name("a &lt;b&gt;"), "a <b>");
    assert_eq!(preset_name("Bob's"), "Bob");
    assert_eq!(preset_name("   "), DEFAULT_IMPORT_PRESET_NAME);
    assert_eq!(preset_name("\u{1C}"), DEFAULT_IMPORT_PRESET_NAME);
    assert_eq!(preset_name("\u{2028}"), DEFAULT_IMPORT_PRESET_NAME);
    assert_eq!(preset_name("\u{A0}"), "\u{A0}");
    let without_meta = format!(
        "<html><body>{}</body></html>",
        row("m", &format!("{WORKSHOP}1"))
    );
    assert_eq!(
        compare_alone(&without_meta).preset_name,
        DEFAULT_IMPORT_PRESET_NAME
    );
    // `content` before `name` is not recognised.
    let reversed = format!(
        "<meta content=\"X\" name=\"arma:PresetName\" />{}",
        row("m", &format!("{WORKSHOP}1"))
    );
    assert_eq!(
        compare_alone(&reversed).preset_name,
        DEFAULT_IMPORT_PRESET_NAME
    );
}

// ----- Row parsing -----

#[test]
fn rows_match_case_insensitively_in_either_attribute_order() {
    let html = concat!(
        "<TR DATA-TYPE='ModContainer'><TD data-type=\"DisplayName\">Upper</TD>",
        "<td><A HREF='https://steamcommunity.com/sharedfiles/filedetails/?id=11' DATA-TYPE='Link'>x</A></td></TR>",
        "<tr data-type = 'ModContainer\"><td data-type=\"DisplayName\">Mixed</td>",
        "<td><a data-type=\"Link\" href=\"https://steamcommunity.com/sharedfiles/filedetails/?ID=12abc\">x</a></td></tr>",
    );

    let result = compare_alone(html);

    let ids: Vec<_> = result
        .missing
        .iter()
        .map(|m| (m.published_id.as_str(), m.name.as_str()))
        .collect();
    assert_eq!(ids, [("11", "Upper"), ("12", "Mixed")]);
}

#[test]
fn workshop_rows() {
    let html = preset(
        "p",
        &[
            row("No id", "https://example.com/mod"),
            row("", &format!("{WORKSHOP}21")),
            row(
                "First",
                "https://steamcommunity.com/sharedfiles/filedetails/?id=22&amp;foo",
            ),
            row("Second", &format!("{WORKSHOP}22")),
        ],
    );

    let result = compare_alone(&html);

    assert_eq!(result.requested_count, 2);
    assert_eq!(result.missing[0].name, "Workshop item 21");
    assert_eq!(result.missing[1].name, "First");
    assert_eq!(result.missing[1].url, format!("{WORKSHOP}22"));
}

#[test]
fn dlc_rows() {
    let html = preset(
        "p",
        &[
            dlc_row("", "https://store.steampowered.com/app/123?x=1"),
            dlc_row("", "https://store.steampowered.com/?appid=1021790"),
            dlc_row("Repeated", "https://store.steampowered.com/app/1021790"),
        ],
    );

    let result = compare_alone(&html);

    assert_eq!(result.requested_dlc_count, 2);
    let unknown = &result.missing_dlc[0];
    assert_eq!(unknown.name, "CDLC 123");
    assert_eq!(unknown.url, "https://store.steampowered.com/app/123?x=1");
    assert_eq!(unknown.addon_key, None);
    let contact = &result.missing_dlc[1];
    assert_eq!(contact.dlc, Some(GameDlc::Contact));
    assert_eq!(contact.name, "Contact");
    assert_eq!(contact.url, "https://store.steampowered.com/app/1021790");
    assert_eq!(contact.addon_key.as_deref(), Some("Contact"));
}

// ----- Local addons -----

#[test]
fn local_workshop_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().to_str().unwrap();
    addon_dir(
        tmp.path(),
        "@Shared",
        Some("publishedid = 31;\nname = \"  Trimmed  \";\n"),
    );
    addon_dir(
        tmp.path(),
        "@second",
        Some("publishedid = 31;\nname = \"Second\";\n"),
    );
    addon_dir(
        tmp.path(),
        "@empty_name",
        Some("publishedid = 32;\nname = \"\";\n"),
    );
    addon_dir(tmp.path(), "@zero", Some("publishedid = 033;\n"));
    let addons = [
        Addon::new("   ", "@Shared", parent),
        // Same folder, other case: read once, so its key is not used for id 31.
        Addon::new("shared-again", "@shared", parent),
        Addon::new("second", "@second", parent),
        Addon::new("empty", "@empty_name", parent),
        Addon::new("zero", "@zero", parent),
    ];
    let html = preset(
        "p",
        &[
            row("a", &format!("{WORKSHOP}31")),
            row("b", &format!("{WORKSHOP}32")),
            row("c", &format!("{WORKSHOP}33")),
        ],
    );

    let result = compare(&html, &addons).unwrap();

    let matched: Vec<_> = result
        .matched
        .iter()
        .map(|m| {
            (
                m.published_id.as_str(),
                m.local_name.as_deref().unwrap(),
                m.local_addon_key.as_deref().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        matched,
        [("31", "Trimmed", "@Shared"), ("32", "@empty_name", "empty")]
    );
    assert_eq!(result.missing[0].published_id, "33");
}

#[test]
fn folders_differing_only_in_case_are_read_once() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().to_str().unwrap();
    addon_dir(tmp.path(), "@mod", Some("publishedid = 41;\n"));
    let dotted = format!("{parent}/./sub/..");
    let addons = [
        Addon::new("first", "@mod", dotted.as_str()),
        Addon::new("second", "@MOD", parent),
    ];
    let result = compare(&preset("p", &[row("m", &format!("{WORKSHOP}41"))]), &addons).unwrap();
    assert_eq!(result.matched[0].local_addon_key.as_deref(), Some("first"));
}

#[test]
fn local_cdlcs_need_their_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().to_str().unwrap();
    fs::create_dir(tmp.path().join("gm")).unwrap();
    let addons = [
        Addon::new("gm", "gm", parent),
        Addon::new("ws", "WS", parent),
    ];
    let html = preset(
        "p",
        &[
            dlc_row("", "https://store.steampowered.com/app/1042220"),
            dlc_row("", "https://store.steampowered.com/app/1681170"),
        ],
    );

    let result = compare(&html, &addons).unwrap();

    assert_eq!(result.matched_dlc.len(), 1);
    assert_eq!(result.matched_dlc[0].local_addon_key.as_deref(), Some("GM"));
    assert_eq!(result.missing_dlc[0].local_addon_key.as_deref(), Some("WS"));
}

#[test]
fn unreadable_meta_cpp_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().to_str().unwrap();
    fs::create_dir(tmp.path().join("@bad")).unwrap();
    fs::write(tmp.path().join("@bad/meta.cpp"), b"publishedid = 51;\xff\n").unwrap();
    addon_dir(tmp.path(), "@good", Some("publishedid = 51;\n"));
    let addons = [
        Addon::new("bad", "@bad", parent),
        Addon::new("good", "@good", parent),
    ];
    let result = compare(&preset("p", &[row("m", &format!("{WORKSHOP}51"))]), &addons).unwrap();
    assert_eq!(result.matched[0].local_addon_key.as_deref(), Some("good"));
}

#[test]
fn import_preset_reads_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing.html");
    assert!(matches!(
        import_preset(&missing, &[]),
        Err(ImportError::NotAFile(path)) if path == missing
    ));
    assert!(matches!(
        import_preset(tmp.path(), &[]),
        Err(ImportError::NotAFile(_))
    ));

    let invalid = tmp.path().join("invalid.html");
    fs::write(&invalid, b"\xff\xfe").unwrap();
    assert!(matches!(
        import_preset(&invalid, &[]),
        Err(ImportError::ReadPreset { .. })
    ));

    let file = tmp.path().join("preset.html");
    fs::write(
        &file,
        preset("From file", &[row("m", &format!("{WORKSHOP}61"))]),
    )
    .unwrap();
    assert_eq!(import_preset(&file, &[]).unwrap().preset_name, "From file");
}
