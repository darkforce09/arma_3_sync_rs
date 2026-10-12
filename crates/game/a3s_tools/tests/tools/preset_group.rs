//! The addon-group helpers around preset import and export.

use std::path::{Path, PathBuf};

use a3s_tools::preset::{
    Cdlc, DEFAULT_EXPORT_PRESET_NAME, DEFAULT_IMPORTED_GROUP_NAME, ImportResult, MetadataIssue,
    MetadataIssueReason, WorkshopMod, add_imported_group, ensure_html_extension, format_messages,
    select_export_modset, suggested_file_name, unique_imported_group_name, validate_metadata_input,
};
use a3s_types::constants::{GameDlc, ModsetType};
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};

fn workshop_mod(id: &str, name: &str, local_key: Option<&str>) -> WorkshopMod {
    WorkshopMod {
        published_id: id.to_owned(),
        name: name.to_owned(),
        url: format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}"),
        local_name: local_key.map(|_| name.to_owned()),
        local_addon_key: local_key.map(str::to_owned),
    }
}

fn cdlc(dlc: GameDlc, local: bool) -> Cdlc {
    Cdlc {
        dlc: Some(dlc),
        addon_key: Some(dlc.java_name().to_owned()),
        name: dlc.display_name().to_owned(),
        steam_app_id: dlc.steam_app_id().to_owned(),
        url: dlc.steam_store_url(),
        local_addon_key: local.then(|| dlc.java_name().to_owned()),
    }
}

fn group(name: &str) -> TreeNode {
    TreeNode::Directory(TreeDirectory::new(name))
}

fn modset(name: &str, selected: bool) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        selected,
        modset_type: Some(ModsetType::Repository),
        ..TreeDirectory::new(name)
    })
}

#[test]
fn imported_group_has_one_sorted_leaf_per_entry() {
    let mut root = TreeDirectory::new_root();
    let result = ImportResult {
        preset_name: "  Event  ".to_owned(),
        requested_count: 2,
        requested_dlc_count: 3,
        matched: vec![workshop_mod("1", "Local", Some("zeta"))],
        missing: vec![workshop_mod("2", "Alpha Missing", None)],
        matched_dlc: vec![cdlc(GameDlc::Gm, true)],
        missing_dlc: vec![
            cdlc(GameDlc::Ws, false),
            Cdlc {
                dlc: None,
                addon_key: None,
                name: "CDLC 123".to_owned(),
                steam_app_id: "123".to_owned(),
                url: "https://store.steampowered.com/app/123".to_owned(),
                local_addon_key: None,
            },
        ],
    };

    let name = add_imported_group(&mut root, &result);

    assert_eq!(name, "Event");
    let [TreeNode::Directory(group)] = root.children.as_slice() else {
        panic!("one group expected");
    };
    assert_eq!(group.name, "Event");
    assert!(group.selected);
    assert_eq!(group.modset_type, None);
    let leaves: Vec<_> = group
        .leaves()
        .map(|leaf| {
            (
                leaf.name.as_str(),
                leaf.workshop_id.as_deref(),
                leaf.dlc_app_id.as_deref(),
                leaf.missing,
            )
        })
        .collect();
    assert_eq!(
        leaves,
        [
            ("Alpha Missing", Some("2"), None, true),
            ("CDLC 123", None, Some("123"), true),
            ("GM", None, Some("1042220"), false),
            ("WS", None, Some("1681170"), true),
            ("zeta", Some("1"), None, false),
        ]
    );
    assert!(group.leaves().all(|leaf| leaf.selected));
}

#[test]
fn imported_group_names_are_unique_among_groups() {
    let mut root = TreeDirectory::new_root();
    root.add_child(group("X"));
    root.add_child(group("x (2)"));
    root.add_child(TreeNode::Leaf(TreeLeaf::new("Y")));

    assert_eq!(unique_imported_group_name(&root, "x"), "x (3)");
    assert_eq!(unique_imported_group_name(&root, " Y "), "Y");
    assert_eq!(
        unique_imported_group_name(&root, " \t"),
        DEFAULT_IMPORTED_GROUP_NAME
    );
}

#[test]
fn export_modset_selection() {
    let mut root = TreeDirectory::new_root();
    root.add_child(group("Plain"));
    root.add_child(modset("Checked", true));
    root.add_child(modset("Other", false));
    // Directories sorted: Checked (0), Other (1), Plain (2).
    let [checked, other, plain] = [0, 1, 2];

    assert_eq!(select_export_modset(&root, Some(&[other, 0])), Some(other));
    assert_eq!(select_export_modset(&root, Some(&[plain])), Some(checked));
    assert_eq!(select_export_modset(&root, Some(&[])), Some(checked));
    assert_eq!(select_export_modset(&root, Some(&[7])), Some(checked));
    assert_eq!(select_export_modset(&root, None), Some(checked));

    root.add_child(modset("Second", true));
    assert_eq!(select_export_modset(&root, None), None);

    let mut only_groups = TreeDirectory::new_root();
    only_groups.add_child(TreeNode::Directory(TreeDirectory {
        selected: true,
        ..TreeDirectory::new("Group")
    }));
    assert_eq!(select_export_modset(&only_groups, Some(&[0])), None);
}

#[test]
fn file_names() {
    assert_eq!(suggested_file_name(" a/b:c "), "a_b_c.html");
    assert_eq!(suggested_file_name("<>:\"/\\|?*"), "_________.html");
    assert_eq!(
        suggested_file_name("   "),
        format!("{DEFAULT_EXPORT_PRESET_NAME}.html")
    );
    assert_eq!(
        ensure_html_extension(Path::new("dir/x.HTML")),
        PathBuf::from("dir/x.HTML")
    );
    assert_eq!(
        ensure_html_extension(Path::new("dir/x.htm")),
        PathBuf::from("dir/x.htm.html")
    );
    assert_eq!(
        ensure_html_extension(Path::new("x")),
        PathBuf::from("x.html")
    );
}

#[test]
fn metadata_dialog_input() {
    assert_eq!(
        validate_metadata_input(" 12 ", " n "),
        Ok(("12".to_owned(), "n".to_owned()))
    );
    assert_eq!(
        validate_metadata_input("99999999999999999999", "n"),
        Ok(("99999999999999999999".to_owned(), "n".to_owned()))
    );
    assert!(validate_metadata_input("012", "n").is_err());
    assert!(validate_metadata_input("", "n").is_err());
    assert!(validate_metadata_input("1a", "n").is_err());
    assert!(validate_metadata_input("12", " ").is_err());

    let issue = MetadataIssue {
        addon_key: "key".to_owned(),
        addon_name: "@folder".to_owned(),
        reason: MetadataIssueReason::MetaCppMissing,
        published_id: None,
        name: " ".to_owned(),
    };
    assert_eq!(issue.prefill(), (String::new(), "@folder".to_owned()));
    let issue = MetadataIssue {
        published_id: Some("0".to_owned()),
        name: "Name".to_owned(),
        ..issue
    };
    assert_eq!(issue.prefill(), ("0".to_owned(), "Name".to_owned()));
}

#[test]
fn messages_are_listed() {
    assert_eq!(
        format_messages(&["a".to_owned(), "b".to_owned()]),
        "- a\n- b"
    );
    assert_eq!(format_messages(&[]), "");
}
