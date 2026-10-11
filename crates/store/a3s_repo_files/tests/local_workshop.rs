//! Workshop metadata cache (A6).

use a3s_repo_files::local::{Error, LocalDirs, LocalStore, WorkshopEntry};

fn store(root: &std::path::Path) -> LocalStore {
    LocalStore::with_dirs(LocalDirs {
        config: root.join("config"),
        profiles: root.join("profiles"),
        repositories: root.join("repositories"),
        sha1_cache: root.join("sha1_cache"),
    })
}

fn rejection(result: Result<(), Error>) -> &'static str {
    match result {
        Err(Error::InvalidWorkshopEntry(message)) => message,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn a6_save_validates_like_java() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let workshop = store.workshop();
    assert_eq!(
        rejection(workshop.save("", "1", "x")),
        "Addon key must not be empty."
    );
    assert_eq!(
        rejection(workshop.save(" \t", "1", "x")),
        "Addon key must not be empty."
    );
    for id in ["0", "01", " 1", "1 ", "x", "", "-1", "١"] {
        assert_eq!(
            rejection(workshop.save("@ace", id, "ACE")),
            "Workshop ID must be a positive number.",
            "{id:?}"
        );
    }
    assert_eq!(
        rejection(workshop.save("@ace", "1", "  ")),
        "Workshop name must not be empty."
    );
    // Key checked before id, id before name.
    assert_eq!(
        rejection(workshop.save("", "x", "")),
        "Addon key must not be empty."
    );
    assert_eq!(
        rejection(workshop.save("@a", "x", "")),
        "Workshop ID must be a positive number."
    );
    assert!(!store.workshop_path().exists());
}

#[test]
fn a6_save_then_find() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let workshop = store.workshop();
    assert_eq!(workshop.find("@ace"), None);
    workshop.save("@ace", "463939057", " ACE ").unwrap();
    assert_eq!(
        workshop.find("@ace"),
        Some(WorkshopEntry {
            addon_key: "@ace".to_owned(),
            published_id: "463939057".to_owned(),
            name: "ACE".to_owned(),
        })
    );
    assert_eq!(workshop.find(""), None);
    assert_eq!(workshop.find("@cba"), None);

    // A second save keeps the first entry.
    workshop.save("@cba", "450814997", "CBA_A3").unwrap();
    let all = store.workshop().all();
    assert_eq!(all.len(), 2);
    assert_eq!(all["@ace"].name, "ACE");
    assert_eq!(all["@cba"].published_id, "450814997");

    // Saving the same key replaces it.
    workshop.save("@ace", "1", "ACE 3").unwrap();
    assert_eq!(workshop.find("@ace").unwrap().name, "ACE 3");
}

#[test]
fn a6_blank_stored_values_and_corrupt_files_read_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        store.workshop_path(),
        "format = 1\n[workshop.mods.\"@a\"]\npublished_id = \"1\"\nname = \" \"\n\
         [workshop.mods.\"@b\"]\npublished_id = \" 2 \"\nname = \"B\"\n",
    )
    .unwrap();
    let workshop = store.workshop();
    assert_eq!(workshop.find("@a"), None);
    assert_eq!(workshop.find("@b").unwrap().published_id, "2");
    assert_eq!(workshop.all().len(), 1);

    // An unreadable file is treated as empty and replaced on the next save.
    std::fs::write(store.workshop_path(), "garbage [").unwrap();
    assert_eq!(workshop.find("@b"), None);
    workshop.save("@c", "3", "C").unwrap();
    assert_eq!(workshop.all().len(), 1);
}

#[test]
fn a6_insert_missing_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let workshop = store.workshop();
    workshop.save("@ace", "1", "Mine").unwrap();
    let entry = |key: &str, id: &str, name: &str| WorkshopEntry {
        addon_key: key.to_owned(),
        published_id: id.to_owned(),
        name: name.to_owned(),
    };
    let added = workshop
        .insert_missing(&[entry("@ace", "2", "Theirs"), entry("@cba", "3", " CBA ")])
        .unwrap();
    assert_eq!(added, vec!["@cba".to_owned()]);
    assert_eq!(workshop.find("@ace").unwrap().name, "Mine");
    assert_eq!(workshop.find("@cba").unwrap().name, "CBA");
}

#[test]
fn a6_insert_missing_leaves_unreadable_files_alone() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    let new = WorkshopEntry {
        addon_key: "@ace".to_owned(),
        published_id: "1".to_owned(),
        name: "N".to_owned(),
    };
    for content in [
        "format = 2\n[workshop.mods.\"@mine\"]\npublished_id = \"42\"\nname = \"Mine\"\n",
        "format = 1\n[workshop.mods.\"@mine\"\n",
    ] {
        std::fs::write(store.workshop_path(), content).unwrap();
        assert!(
            store
                .workshop()
                .insert_missing(std::slice::from_ref(&new))
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(store.workshop_path()).unwrap(),
            content
        );
    }
}
