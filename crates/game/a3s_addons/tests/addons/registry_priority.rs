//! The addon registry's key allocation and duplicate lookup, and the priority lists.

use a3s_addons::priority::{
    move_down, move_top, move_up, reset_addon_priority, sync_addon_priorities,
};
use a3s_addons::{Addon, AddonRegistry};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn keys_get_a_star_per_earlier_addon_of_the_same_name_ignoring_case() {
    let mut registry = AddonRegistry::new();
    let keys: Vec<String> = ["@ace", "@ace", "@ACE", "@ace"]
        .iter()
        .map(|name| registry.register(name, "/mods".to_owned()))
        .collect();
    assert_eq!(keys, strings(&["@ace", "@ace*", "@ACE**", "@ace***"]));
    assert_eq!(registry.len(), 4);

    let second = registry.get("@ACE*").unwrap();
    assert_eq!(
        (second.key.as_str(), second.name.as_str()),
        ("@ace*", "@ace")
    );

    assert_eq!(
        registry.duplicates("@ace"),
        strings(&["@ace", "@ace*", "@ace***"])
    );
    assert!(registry.has_duplicate("@ace*"));
    assert!(!registry.has_duplicate("@ACE**"));
    assert!(registry.duplicates("nope").is_empty());
    assert!(!registry.has_duplicate("nope"));
    assert_eq!(
        registry.keys().collect::<Vec<_>>(),
        ["@ace", "@ace*", "@ace**", "@ace***"]
    );

    registry.clear();
    assert!(registry.is_empty());
    assert_eq!(registry.determine_new_key("@ace"), "@ace");
}

#[test]
fn insert_replaces_an_existing_key_in_place() {
    let mut registry = AddonRegistry::new();
    registry.insert(Addon::new("@a", "@a", "/one"));
    registry.insert(Addon::new("@b", "@b", "/one"));
    registry.insert(Addon::new("@A", "@A", "/two"));
    assert_eq!(registry.keys().collect::<Vec<_>>(), ["@a", "@b"]);
    assert_eq!(registry.get("@a").unwrap().path, "/two");
}

fn registry_of(keys: &[&str]) -> AddonRegistry {
    let mut registry = AddonRegistry::new();
    for key in keys {
        registry.insert(Addon::new(*key, *key, "/mods"));
    }
    registry
}

#[test]
fn priorities_follow_the_registry() {
    let registry = registry_of(&["@B", "@a", "gm"]);

    let mut list = Vec::new();
    assert!(sync_addon_priorities(&mut list, &registry));
    assert_eq!(list, strings(&["@a", "@b", "gm"]));
    assert!(!sync_addon_priorities(&mut list, &registry));

    let mut list = strings(&["gm", "@zzz"]);
    assert!(sync_addon_priorities(&mut list, &registry));
    assert_eq!(list, strings(&["gm", "@a", "@b"]));

    let mut list = strings(&["gm"]);
    assert!(sync_addon_priorities(&mut list, &AddonRegistry::new()));
    assert!(list.is_empty());

    let mut list = strings(&["gm"]);
    assert!(reset_addon_priority(&mut list));
    assert!(list.is_empty());
    assert!(!reset_addon_priority(&mut list));
}

#[test]
fn moves_ignore_boundaries() {
    let original = strings(&["a", "b", "c"]);
    let mut list = original.clone();

    assert!(!move_top(&mut list, 0));
    assert!(!move_top(&mut list, 3));
    assert!(!move_up(&mut list, 0));
    assert!(!move_up(&mut list, 3));
    assert!(!move_down(&mut list, 2));
    assert!(!move_down(&mut list, 3));
    assert!(!move_down(&mut list, usize::MAX));
    assert_eq!(list, original);

    let mut empty: Vec<String> = Vec::new();
    assert!(!move_top(&mut empty, 0));
    assert!(!move_up(&mut empty, 0));
    assert!(!move_down(&mut empty, 0));

    assert!(move_top(&mut list, 2));
    assert_eq!(list, strings(&["c", "a", "b"]));
    assert!(move_up(&mut list, 2));
    assert_eq!(list, strings(&["c", "b", "a"]));
    assert!(move_down(&mut list, 0));
    assert_eq!(list, strings(&["b", "c", "a"]));
}
