//! Preset export: modset traversal, metadata fallbacks, the Java-identical HTML and writing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_tools::preset::{
    DEFAULT_EXPORT_PRESET_NAME, ExportResult, MetadataIssueReason, PresetEntry, WorkshopEntry,
    WorkshopMetadataLookup, WriteError, compare, escape_html, generate, render_html, write,
};
use a3s_types::local::{Addon, TreeDirectory, TreeLeaf, TreeNode};
use tempfile::TempDir;

/// Addon folders in a temporary directory, resolved by lower-case name like the Java tests.
struct Addons {
    tmp: TempDir,
    by_key: HashMap<String, Addon>,
}

impl Addons {
    fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
            by_key: HashMap::new(),
        }
    }

    fn parent(&self) -> &Path {
        self.tmp.path()
    }

    /// Adds the addon folder `name`, with `meta.cpp` when given.
    fn add(&mut self, name: &str, meta: Option<&str>) -> &mut Self {
        self.add_bytes(name, meta.map(str::as_bytes))
    }

    fn add_bytes(&mut self, name: &str, meta: Option<&[u8]>) -> &mut Self {
        let dir = self.parent().join(name);
        fs::create_dir_all(&dir).unwrap();
        if let Some(meta) = meta {
            fs::write(dir.join("meta.cpp"), meta).unwrap();
        }
        let addon = Addon::new(name, name, self.parent().to_str().unwrap());
        self.by_key.insert(name.to_lowercase(), addon);
        self
    }

    fn generate(&self, modset: &TreeDirectory) -> ExportResult {
        generate(
            modset,
            &|name: &str| self.by_key.get(&name.to_lowercase()),
            None,
        )
    }

    fn generate_with(
        &self,
        modset: &TreeDirectory,
        known: &dyn WorkshopMetadataLookup,
    ) -> ExportResult {
        generate(
            modset,
            &|name: &str| self.by_key.get(&name.to_lowercase()),
            Some(known),
        )
    }
}

fn leaf(name: &str, selected: bool) -> TreeNode {
    TreeNode::Leaf(TreeLeaf {
        selected,
        ..TreeLeaf::new(name)
    })
}

fn modset(name: &str, children: Vec<TreeNode>) -> TreeDirectory {
    let mut modset = TreeDirectory::new(name);
    for child in children {
        modset.add_child(child);
    }
    modset
}

fn entry(published_id: &str, name: &str) -> WorkshopEntry {
    WorkshopEntry {
        addon_key: String::new(),
        published_id: published_id.to_owned(),
        name: name.to_owned(),
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_presets")
}

// ----- Java tests -----

#[test]
fn generates_workshop_and_dlc_entries_from_selected_modset() {
    let mut addons = Addons::new();
    addons.add(
        "GM Enhancement",
        Some("protocol = 1;\npublishedid = 3147580028;\nname = \"GM Enhancement & Intern\";\ntimestamp = 5250609374435198798;\n"),
    );
    let modset = modset(
        "Event <HS13>",
        vec![leaf("GM Enhancement", true), leaf("WS", true)],
    );

    let result = addons.generate(&modset);

    assert!(result.is_valid());
    let html = result.html().unwrap();
    assert!(html.contains("data-type=\"ModContainer\""));
    assert!(html.contains("id=3147580028"));
    assert!(html.contains("GM Enhancement &amp; Intern"));
    assert!(html.contains("data-type=\"DlcContainer\""));
    assert!(html.contains("store.steampowered.com/app/1681170"));
    // The whole document is Java's.
    let golden = fs::read_to_string(fixtures().join("p1_golden/preset.html")).unwrap();
    assert_eq!(html, golden);
}

#[test]
fn reports_missing_meta_cpp_as_metadata_issue() {
    let mut addons = Addons::new();
    addons.add("Missing Metadata", None).add(
        "Valid Mod",
        Some("publishedid = 3147580028;\nname = \"Valid Mod\";\n"),
    );
    let modset = modset(
        "Missing metadata",
        vec![leaf("Missing Metadata", true), leaf("Valid Mod", true)],
    );

    let result = addons.generate(&modset);

    assert!(!result.is_valid());
    assert_eq!(result.html(), None);
    let issues = result.metadata_issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].reason, MetadataIssueReason::MetaCppMissing);
    assert_eq!(issues[0].addon_key, "Missing Metadata");
    assert_eq!(issues[0].published_id, None);
    assert_eq!(result.workshop_count(), 1);
    assert!(result.errors().is_empty());
}

#[test]
fn uses_known_metadata_when_meta_cpp_has_no_name() {
    let mut addons = Addons::new();
    addons.add(
        "Folder Name Fallback",
        Some("protocol = 1;\npublishedid = 3147580028;\ntimestamp = 5250609374435198798;\n"),
    );
    let modset = modset("Fallback", vec![leaf("Folder Name Fallback", true)]);
    let dirs = addons.parent().join("store");
    let store = LocalStore::with_dirs(LocalDirs {
        config: dirs.join("config"),
        profiles: dirs.join("profiles"),
        repositories: dirs.join("repositories"),
        sha1_cache: dirs.join("sha1_cache"),
    });
    store
        .workshop()
        .save("Folder Name Fallback", "3147580028", "Known Display Name")
        .unwrap();

    let result = addons.generate_with(&modset, &store.workshop());

    assert!(result.is_valid());
    assert!(result.html().unwrap().contains("Known Display Name"));
}

#[test]
fn uses_addon_folder_name_when_meta_cpp_has_no_name_and_store_is_missing() {
    let mut addons = Addons::new();
    addons.add("ACRE2", Some("protocol = 1;\npublishedid = 751965892;\n"));
    let modset = modset("No metadata database", vec![leaf("ACRE2", true)]);
    let dirs = addons.parent().join("no-store");
    let store = LocalStore::with_dirs(LocalDirs {
        config: dirs.join("config"),
        profiles: dirs.join("profiles"),
        repositories: dirs.join("repositories"),
        sha1_cache: dirs.join("sha1_cache"),
    });

    let result = addons.generate_with(&modset, &store.workshop());

    assert!(result.is_valid());
    assert!(result.metadata_issues().is_empty());
    assert!(result.html().unwrap().contains("ACRE2"));
}

#[test]
fn treats_zero_published_id_as_missing_metadata() {
    let mut addons = Addons::new();
    addons.add(
        "Zero ID Mod",
        Some("protocol = 1;\npublishedid = 0;\nname = \"Zero ID Mod\";\n"),
    );
    let modset = modset("Zero ID preset", vec![leaf("Zero ID Mod", true)]);

    let result = addons.generate(&modset);

    assert!(!result.is_valid());
    let issue = &result.metadata_issues()[0];
    assert_eq!(issue.reason, MetadataIssueReason::PublishedIdInvalid);
    assert_eq!(issue.published_id.as_deref(), Some("0"));
}

#[test]
fn writes_the_generated_preset_to_disk() {
    let mut addons = Addons::new();
    addons.add(
        "Writable Mod",
        Some("publishedid = 3147580028;\nname = \"Writable Mod\";\n"),
    );
    let result = addons.generate(&modset("Writable preset", vec![leaf("Writable Mod", true)]));
    let target = addons.parent().join("nested").join("preset.html");

    write(&target, &result).unwrap();

    assert!(target.is_file());
    assert!(fs::metadata(&target).unwrap().len() > 0);
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("Writable Mod")
    );
}

#[test]
fn replaces_an_existing_preset_on_disk() {
    let mut addons = Addons::new();
    addons.add(
        "Overwrite Mod",
        Some("publishedid = 3147580028;\nname = \"Overwrite Mod\";\n"),
    );
    let result = addons.generate(&modset(
        "Overwrite preset",
        vec![leaf("Overwrite Mod", true)],
    ));
    let target = addons.parent().join("preset.html");
    fs::write(&target, "old content").unwrap();

    write(&target, &result).unwrap();

    let written = fs::read_to_string(&target).unwrap();
    assert!(written.contains("Overwrite Mod"));
    assert!(!written.contains("old content"));
}

// ----- Traversal -----

#[test]
fn traversal_follows_leaf_selection_only() {
    let mut addons = Addons::new();
    addons
        .add("@inner", Some("publishedid = 11;\nname = \"Inner\";\n"))
        .add("@skipped", Some("publishedid = 12;\nname = \"Skipped\";\n"))
        .add("@first", Some("publishedid = 13;\nname = \"First\";\n"))
        .add("@second", Some("publishedid = 13;\nname = \"Second\";\n"));
    let group = modset("Group", vec![leaf("@inner", true)]);
    let modset = modset(
        "Traversal",
        vec![
            TreeNode::Directory(group),
            leaf("@skipped", false),
            leaf("@first", true),
            leaf("@second", true),
            leaf("ws", true),
            leaf("WS", true),
        ],
    );

    let result = addons.generate(&modset);

    assert!(result.is_valid());
    assert_eq!(
        result.workshop_entries(),
        [
            PresetEntry {
                name: "Inner".to_owned(),
                url: "https://steamcommunity.com/sharedfiles/filedetails/?id=11".to_owned(),
            },
            PresetEntry {
                name: "First".to_owned(),
                url: "https://steamcommunity.com/sharedfiles/filedetails/?id=13".to_owned(),
            },
        ]
    );
    assert_eq!(result.warnings().len(), 1);
    assert!(result.warnings()[0].ends_with(" 13"));
    assert_eq!(result.dlc_count(), 1);
}

#[test]
fn unresolved_leaf_is_an_error_and_the_rest_is_processed() {
    let mut addons = Addons::new();
    addons.add("@here", Some("publishedid = 21;\n"));
    let modset = modset(
        "Unresolved",
        vec![leaf("@absent", true), leaf("@here", true), leaf("GM", true)],
    );

    let result = addons.generate(&modset);

    assert!(!result.is_valid());
    assert_eq!(result.html(), None);
    assert_eq!(result.errors().len(), 1);
    assert!(result.errors()[0].ends_with(" @absent"));
    assert_eq!(result.workshop_count(), 1);
    assert_eq!(result.dlc_count(), 1);
}

#[test]
fn empty_modset_is_an_error() {
    let result = Addons::new().generate(&modset("", vec![leaf("@off", false)]));
    assert_eq!(result.errors().len(), 1);
    assert_eq!(result.preset_name(), DEFAULT_EXPORT_PRESET_NAME);
    assert!(!result.is_valid());
}

// ----- Metadata -----

#[test]
fn metadata_fallbacks() {
    let mut addons = Addons::new();
    addons
        .add(
            "@huge",
            Some("publishedid = 9223372036854775808;\nname = \"Huge\";\n"),
        )
        .add("@max", Some("publishedid = 9223372036854775807;\n"))
        .add(
            "@stored_id",
            Some("publishedid = 0;\nname = \"Own Name\";\n"),
        )
        .add("@empty_name", Some("publishedid = 31;\nname = \"\";\n"))
        .add("@stored_name", Some("publishedid = 32;\n"));
    let store = |key: &str| match key {
        "@huge" | "@max" => None,
        "@stored_id" => Some(entry("30", "Stored Name")),
        _ => Some(entry("99", "Stored Name")),
    };
    let modset = modset(
        "Metadata",
        vec![
            leaf("@huge", true),
            leaf("@max", true),
            leaf("@stored_id", true),
            leaf("@empty_name", true),
            leaf("@stored_name", true),
        ],
    );

    let result = addons.generate_with(&modset, &store);

    let issue = &result.metadata_issues()[0];
    assert_eq!(result.metadata_issues().len(), 1);
    assert_eq!(issue.reason, MetadataIssueReason::PublishedIdInvalid);
    assert_eq!(issue.published_id.as_deref(), Some("9223372036854775808"));
    assert_eq!(issue.name, "Huge");
    let entries: Vec<_> = result
        .workshop_entries()
        .iter()
        .map(|e| (e.url.rsplit('=').next().unwrap(), e.name.as_str()))
        .collect();
    assert_eq!(
        entries,
        [
            ("31", "@empty_name"),
            ("9223372036854775807", "@max"),
            ("30", "Own Name"),
            ("32", "Stored Name"),
        ]
    );
}

#[test]
fn metadata_issue_alone_also_reports_no_entries() {
    let mut addons = Addons::new();
    addons.add("@zero", Some("publishedid = 0;\n"));

    let result = addons.generate(&modset("Only issue", vec![leaf("@zero", true)]));

    assert_eq!(result.metadata_issues().len(), 1);
    assert_eq!(result.metadata_issues()[0].name, "@zero");
    assert_eq!(result.errors().len(), 1);
}

#[test]
fn unreadable_meta_cpp_is_an_error() {
    let mut addons = Addons::new();
    addons.add_bytes("@bad", Some(b"publishedid = 41;\xff\n"));

    let result = addons.generate(&modset("Unreadable", vec![leaf("@bad", true)]));

    let error = &result.errors()[0];
    assert_eq!(result.errors().len(), 1);
    let meta_file = addons.parent().join("@bad").join("meta.cpp");
    assert!(error.starts_with(&format!("Could not read {}: ", meta_file.display())));
    assert!(result.metadata_issues().is_empty());
}

// ----- HTML -----

#[test]
fn render_html_matches_java() {
    let html = render_html(
        "Event <HS13>",
        &[PresetEntry {
            name: "GM Enhancement & Intern".to_owned(),
            url: "https://steamcommunity.com/sharedfiles/filedetails/?id=3147580028".to_owned(),
        }],
        &[PresetEntry {
            name: "Western Sahara".to_owned(),
            url: "https://store.steampowered.com/app/1681170".to_owned(),
        }],
    );
    let golden = fs::read_to_string(fixtures().join("p1_golden/preset.html")).unwrap();
    assert_eq!(html, golden);

    let without_dlc = render_html("x", &[], &[]);
    assert!(without_dlc.contains("<div class=\"mod-list\">\n      <table>\n      </table>"));
    assert!(!without_dlc.contains("dlc-list\">"));
    assert_eq!(escape_html("a'\"<>&"), "a&#39;&quot;&lt;&gt;&amp;");
}

/// One Java-written case of `tests/fixtures/java_presets`: the modset, the addon folders, and
/// what Java's exporter reported and wrote.
fn run_fixture(case_dir: &Path) {
    let case: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(case_dir.join("case.json")).unwrap()).unwrap();
    let mut addons = Addons::new();
    let mut resolvable = HashMap::new();
    for addon in case["addons"].as_array().unwrap() {
        let folder = addon["folder"].as_str().unwrap();
        addons.add(folder, addon["meta_cpp"].as_str());
        resolvable.insert(
            folder.to_owned(),
            addons.by_key[&folder.to_lowercase()].clone(),
        );
    }
    let modset = match tree(&case["modset"]) {
        TreeNode::Directory(directory) => directory,
        TreeNode::Leaf(_) => panic!("the modset is a directory"),
    };

    // The harness resolves by exact folder name.
    let result = generate(&modset, &|name: &str| resolvable.get(name), None);

    let mut report = format!(
        "valid={}\nworkshop={}\ndlc={}\n",
        result.is_valid(),
        result.workshop_count(),
        result.dlc_count()
    );
    for error in result.errors() {
        report.push_str(&format!("error: {error}\n"));
    }
    for warning in result.warnings() {
        report.push_str(&format!("warning: {warning}\n"));
    }
    for issue in result.metadata_issues() {
        report.push_str(&format!(
            "issue: {}|{}|{}|{}|{}\n",
            issue.addon_key,
            issue.addon_name,
            issue.reason,
            issue.published_id.as_deref().unwrap_or("null"),
            issue.name
        ));
    }
    let expected = fs::read_to_string(case_dir.join("result.txt")).unwrap();
    assert_eq!(report, expected, "{}", case_dir.display());

    let java_html = case_dir.join("preset.html");
    if !java_html.exists() {
        assert_eq!(result.html(), None);
        return;
    }
    let java_html = fs::read(java_html).unwrap();
    let target = addons.parent().join("out").join("preset.html");
    write(&target, &result).unwrap();
    assert_eq!(
        fs::read(&target).unwrap(),
        java_html,
        "{}",
        case_dir.display()
    );

    // Java's file imports back into the same Workshop ids, all present locally.
    let local: Vec<Addon> = resolvable.into_values().collect();
    let imported = compare(&String::from_utf8(java_html).unwrap(), &local).unwrap();
    assert_eq!(imported.preset_name, modset.name);
    assert!(imported.missing.is_empty());
    assert_eq!(imported.requested_count, result.workshop_count());
    assert_eq!(imported.requested_dlc_count, result.dlc_count());
    let urls: Vec<_> = imported.matched.iter().map(|m| m.url.as_str()).collect();
    let exported: Vec<_> = result
        .workshop_entries()
        .iter()
        .map(|e| e.url.as_str())
        .collect();
    assert_eq!(urls, exported);
}

fn tree(node: &serde_json::Value) -> TreeNode {
    let selected = node["selected"].as_bool().unwrap();
    if let Some(name) = node["leaf"].as_str() {
        return leaf(name, selected);
    }
    let mut directory = TreeDirectory::new(node["directory"].as_str().unwrap());
    directory.selected = selected;
    // Stored order, as Java's `getList` dumped it.
    directory.children = node["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(tree)
        .collect();
    TreeNode::Directory(directory)
}

#[test]
fn java_fixtures() {
    let mut cases: Vec<_> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 7);
    for case in cases {
        run_fixture(&case);
    }
}

// ----- Writing -----

fn valid_result(addons: &mut Addons) -> ExportResult {
    addons.add(
        "@mod",
        Some("publishedid = 51;\nname = \"Mod 'quoted' \\\"x\\\"\";\n"),
    );
    addons.generate(&modset(
        "Round <trip>",
        vec![leaf("@mod", true), leaf("GM", true)],
    ))
}

fn temporary_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
        .collect()
}

#[test]
fn invalid_result_is_not_written() {
    let addons = Addons::new();
    let result = addons.generate(&modset("Empty", vec![]));
    let target = addons.parent().join("sub").join("preset.html");

    assert!(matches!(
        write(&target, &result),
        Err(WriteError::InvalidResult)
    ));
    assert!(!target.parent().unwrap().exists());
}

#[test]
fn failed_rename_leaves_no_temporary_file() {
    let mut addons = Addons::new();
    let result = valid_result(&mut addons);
    let out = addons.parent().join("out");
    let target = out.join("preset.html");
    fs::create_dir_all(target.join("occupied")).unwrap();

    assert!(matches!(write(&target, &result), Err(WriteError::Io(_))));
    assert!(temporary_files(&out).is_empty());

    let other = out.join("other.html");
    write(&other, &result).unwrap();
    assert!(temporary_files(&out).is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&other).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn exported_preset_imports_back() {
    let mut addons = Addons::new();
    let result = valid_result(&mut addons);
    let local: Vec<Addon> = addons.by_key.values().cloned().collect();

    let imported = compare(result.html().unwrap(), &local).unwrap();

    assert_eq!(imported.preset_name, "Round <trip>");
    assert_eq!(imported.matched.len(), 1);
    assert_eq!(imported.matched[0].name, "Mod 'quoted' \"x\"");
    assert_eq!(
        imported.matched[0].local_name.as_deref(),
        Some("Mod 'quoted' \"x\"")
    );
    assert_eq!(imported.missing_dlc[0].addon_key.as_deref(), Some("GM"));
}
