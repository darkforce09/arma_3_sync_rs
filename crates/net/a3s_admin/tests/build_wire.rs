//! Graph-level tests: changelog rules, the Java `HashMap` model, instance sharing, carry-over of
//! old changelogs and events.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_admin::build::testing::{
    JavaHashMap, RawGraph, SyncEmit, plan_changelogs, plan_events, spread_hash,
};
use a3s_jserial::jdk::{Builder, hash_map_capacity, hash_map_order, java_string_hash};
use a3s_jserial::{Graph, NodeId, Value};
use a3s_types::{
    Changelog, Changelogs, Events, JavaCodec, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode, wire,
};

const DATE: i64 = 1_704_067_200_000;

fn wog() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos/wog_ftp")
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn dir(name: &str, addon: bool, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        name: name.to_owned(),
        mark_as_addon: addon,
        children,
        ..Default::default()
    })
}

fn leaf(name: &str, sha1: &str) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: Some(sha1.to_owned()),
        size: 1,
        ..Default::default()
    })
}

fn root(children: Vec<SyncTreeNode>) -> SyncTreeDirectory {
    let mut root = SyncTreeDirectory::new_root();
    root.children = children;
    root
}

fn raw<T: JavaCodec>(value: &T) -> RawGraph {
    a3s_types::decode_stream(&a3s_types::encode_stream(value).unwrap()).unwrap()
}

fn emit<T: JavaCodec>(value: &T) -> (Vec<u8>, Graph) {
    let bytes = a3s_types::encode_stream(value).unwrap();
    let graph = a3s_jserial::read(&bytes).unwrap();
    (bytes, graph)
}

fn changelog(revision: i32, addons: &[&str]) -> Changelog {
    Changelog {
        revision,
        build_date: Some(DATE + i64::from(revision)),
        content_updated: true,
        addons: addons.iter().map(|&a| a.to_owned()).collect(),
        ..Default::default()
    }
}

fn decoded(value: &impl JavaCodec) -> Changelogs {
    a3s_types::decode_stream(&a3s_types::encode_stream(value).unwrap()).unwrap()
}

fn list(graph: &Graph, value: Value) -> Vec<Value> {
    a3s_jserial::jdk::list(graph, value).unwrap().unwrap()
}

fn field(graph: &Graph, object: Value, name: &str) -> Value {
    graph.field(object.as_ref().unwrap(), name).unwrap()
}

fn strings(graph: &Graph, values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|v| graph.opt_string(*v).unwrap().unwrap().to_owned())
        .collect()
}

/// Old and new trees for the content-change case: `@a`, `@Upper` and `group/@nested` modified,
/// `@b` removed, `@c` added.
fn change_trees() -> (SyncTreeDirectory, SyncTreeDirectory) {
    let old = root(vec![
        dir("@a", true, vec![leaf("x", "1")]),
        dir("@b", true, vec![leaf("x", "1")]),
        dir("@Upper", true, vec![leaf("x", "1")]),
        dir(
            "group",
            false,
            vec![dir("@nested", true, vec![leaf("x", "1")])],
        ),
    ]);
    let new = root(vec![
        dir("@a", true, vec![leaf("x", "2")]),
        dir("@c", true, vec![leaf("x", "1")]),
        dir("@Upper", true, vec![leaf("x", "1"), leaf("y", "1")]),
        dir(
            "group",
            false,
            vec![dir("@nested", true, vec![leaf("x", "2")])],
        ),
    ]);
    (old, new)
}

/// Keys in the iteration order of a `new HashMap` filled by sequential `put`s.
fn put_order(keys: &[&str]) -> Vec<String> {
    let mut map = JavaHashMap::new();
    for k in keys {
        map.put((*k).to_owned(), (), ());
    }
    map.iter().map(|e| e.key.clone()).collect()
}

#[test]
fn first_changelog() {
    let sync = root(vec![
        dir("@a", true, vec![leaf("x", "1")]),
        dir("@b", true, vec![]),
    ]);
    let first = decoded(&plan_changelogs(None, None, &sync, 1, DATE, true).unwrap());
    assert_eq!(first.list.len(), 1);
    let c = &first.list[0];
    assert_eq!(
        (c.revision, c.build_date, c.content_updated),
        (1, Some(DATE), true)
    );
    assert_eq!(c.addons, ["@a", "@b"]);
    assert_eq!(c.new_addons, c.addons);
    assert!(c.deleted_addons.is_empty() && c.updated_addons.is_empty());

    // serverinfo kept (revision 5) but no changelogs file: one changelog, no new addons, also
    // without a content change.
    let lost = decoded(&plan_changelogs(None, Some(&sync), &sync, 5, DATE, false).unwrap());
    assert_eq!(lost.list.len(), 1);
    assert_eq!(lost.list[0].revision, 5);
    assert!(!lost.list[0].content_updated);
    assert!(lost.list[0].new_addons.is_empty());

    // An empty old list behaves like no file.
    let empty = raw(&Changelogs::default());
    let from_empty = decoded(&plan_changelogs(Some(&empty), None, &sync, 2, DATE, false).unwrap());
    assert_eq!(from_empty.list.len(), 1);
}

#[test]
fn content_change_new_deleted_updated() {
    let (old_sync, new_sync) = change_trees();
    let old = raw(&Changelogs {
        list: vec![changelog(1, &["@a", "@b", "@Upper", "@nested"])],
    });
    let result =
        decoded(&plan_changelogs(Some(&old), Some(&old_sync), &new_sync, 2, DATE, true).unwrap());
    assert_eq!(result.list.len(), 2);
    let c = &result.list[1];
    assert_eq!(c.revision, 2);
    // Nested addons by name in `addons`, by lowercase path in `updatedAddons`.
    assert_eq!(c.addons, ["@a", "@c", "@Upper", "@nested"]);
    assert_eq!(c.new_addons, ["@c"]);
    assert_eq!(c.deleted_addons, ["@b"]);
    assert_eq!(
        c.updated_addons,
        put_order(&["@a", "@c", "@upper", "group/@nested"])
            .into_iter()
            .filter(|k| k != "@c")
            .collect::<Vec<_>>()
    );

    // Without an old sync, no updated addons.
    let no_sync = decoded(&plan_changelogs(Some(&old), None, &new_sync, 2, DATE, true).unwrap());
    assert!(no_sync.list[1].updated_addons.is_empty());
}

#[test]
fn trimming_keeps_at_most_one_less_than_before() {
    let sync = root(vec![dir("@a", true, vec![])]);
    for (old_len, new_len) in [(9, 10), (10, 10), (12, 12)] {
        let old_list: Vec<Changelog> = (1..=old_len).map(|r| changelog(r, &["@a"])).collect();
        let old = raw(&Changelogs {
            list: old_list.clone(),
        });
        let result =
            decoded(&plan_changelogs(Some(&old), None, &sync, old_len + 1, DATE, true).unwrap());
        assert_eq!(
            result.list.len(),
            usize::try_from(new_len).unwrap(),
            "{old_len}"
        );
        let carried = &result.list[..result.list.len() - 1];
        let dropped = old_list.len() + 1 - result.list.len();
        assert_eq!(carried, &old_list[dropped..], "{old_len}");
        assert_eq!(result.list.last().unwrap().revision, old_len + 1);
    }
}

#[test]
fn no_content_change_keeps_the_list_byte_identical() {
    let sync = root(vec![dir("@a", true, vec![])]);
    let old_value = Changelogs {
        list: vec![changelog(1, &["@a", "@b"]), changelog(2, &["@a"])],
    };
    let old_bytes = a3s_types::encode_stream(&old_value).unwrap();
    let old: RawGraph = a3s_types::decode_stream(&old_bytes).unwrap();
    let plan = plan_changelogs(Some(&old), Some(&sync), &sync, 2, DATE, false).unwrap();
    assert!(a3s_types::encode_stream(&plan).unwrap() == old_bytes);
}

const WOG_ORIGINAL_ORDER: [&str; 14] = [
    "@RHSUSAF",
    "@RHSAFRF",
    "@RHSSAF",
    "@YuEmod",
    "@TFAR",
    "@RHSGREF",
    "@WOG_Islands",
    "@WOG_Mods",
    "@CUP_Terrains_Core",
    "@WOG_Extra",
    "@CUP_Terrains_Maps",
    "@WOG_BW",
    "@WOG_BAF",
    "@WOG_ACE",
];

const WOG_LOWERCASE_ORDER: [&str; 14] = [
    "@cup_terrains_core",
    "@wog_ace",
    "@wog_baf",
    "@rhsgref",
    "@wog_mods",
    "@wog_bw",
    "@yuemod",
    "@cup_terrains_maps",
    "@wog_extra",
    "@rhsusaf",
    "@wog_islands",
    "@rhsafrf",
    "@rhssaf",
    "@tfar",
];

/// Addon names of the WOG sync in `getAddons` order.
fn wog_addons() -> Vec<String> {
    fn rec(dir: &SyncTreeDirectory, out: &mut Vec<String>) {
        for d in dir.directories() {
            if d.mark_as_addon {
                out.push(d.name.clone());
            } else {
                rec(d, out);
            }
        }
    }
    let sync: SyncTreeDirectory = a3s_repo_files::read_a3s(&wog().join("sync")).unwrap();
    let mut out = Vec::new();
    rec(&sync, &mut out);
    out
}

#[test]
fn hash_map_model_reproduces_wog_orders() {
    let addons = wog_addons();
    assert_eq!(addons.len(), 14);
    let names: Vec<&str> = addons.iter().map(String::as_str).collect();
    assert_eq!(put_order(&names), WOG_ORIGINAL_ORDER);
    let lower: Vec<String> = addons.iter().map(|a| a.to_lowercase()).collect();
    let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
    assert_eq!(put_order(&lower), WOG_LOWERCASE_ORDER);

    // The WOG fixture's last changelog (pre-50270ca keys: original case) lists its updated
    // addons in that order.
    let changelogs: Changelogs = a3s_repo_files::read_a3s(&wog().join("changelogs")).unwrap();
    assert_eq!(
        changelogs.list.last().unwrap().updated_addons,
        ["@WOG_Islands", "@WOG_Mods", "@WOG_ACE"]
    );
}

#[test]
fn hash_map_capacities() {
    let keys: Vec<String> = (0..25).map(|i| format!("key{i}")).collect();
    // n → (putAll table/threshold, readObject table, sequential-put table)
    for (n, put_all, read, seq) in [
        (1, (2, 1), 16, 16),
        (2, (4, 3), 16, 16),
        (4, (8, 6), 16, 16),
        (6, (8, 6), 16, 16),
        (7, (16, 12), 16, 16),
        (12, (16, 12), 16, 16),
        (13, (32, 24), 32, 32),
        (24, (32, 24), 32, 32),
        (25, (64, 48), 64, 64),
    ] {
        let entries = || keys[..n].iter().map(|k| (k.clone(), (), ()));
        let mut all: JavaHashMap<(), ()> = JavaHashMap::new();
        all.put_all(entries());
        assert_eq!(all.write_params(), put_all, "putAll {n}");
        let de: JavaHashMap<(), ()> = JavaHashMap::deserialized(entries());
        assert_eq!(de.table_len(), read, "readObject {n}");
        let mut seq_map: JavaHashMap<(), ()> = JavaHashMap::new();
        for (k, p, v) in entries() {
            seq_map.put(k, p, v);
        }
        assert_eq!(seq_map.table_len(), seq, "put {n}");
        assert_eq!(seq_map.table_len(), hash_map_capacity(n), "put {n}");
    }
    // An empty map writes 16 buckets and threshold 0.
    assert_eq!(JavaHashMap::<(), ()>::new().write_params(), (16, 0));
}

#[test]
fn resize_with_collisions_keeps_bucket_then_insertion_order() {
    // Keys colliding in a 16-bucket table but split by the resize to 32 and 64.
    let mut colliding: Vec<String> = (0..10_000)
        .map(|i| format!("c{i}"))
        .filter(|k| spread_hash(k) & 15 == 3)
        .take(6)
        .collect();
    let fillers: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
    colliding.extend(fillers);
    let keys: Vec<&str> = colliding.iter().map(String::as_str).collect();
    let hashes: Vec<i32> = keys.iter().map(|k| java_string_hash(k)).collect();
    let expected: Vec<String> = hash_map_order(&hashes, hash_map_capacity(keys.len()))
        .into_iter()
        .map(|i| keys[i].to_owned())
        .collect();
    assert_eq!(put_order(&keys), expected);
}

#[test]
fn changelog_sharing() {
    let (old_sync, new_sync) = change_trees();
    let old = raw(&Changelogs {
        list: vec![changelog(1, &["@a", "@b", "@Upper", "@nested"])],
    });
    let plan = plan_changelogs(Some(&old), Some(&old_sync), &new_sync, 2, DATE, true).unwrap();
    let (_, graph) = emit(&plan);
    let root_value = graph.root_value().unwrap();
    let entries = list(&graph, field(&graph, root_value, "list"));
    let prev_addons = list(&graph, field(&graph, entries[0], "addons"));
    let addons = list(&graph, field(&graph, entries[1], "addons"));
    let new_addons = list(&graph, field(&graph, entries[1], "newAddons"));
    let deleted = list(&graph, field(&graph, entries[1], "deletedAddons"));
    let updated = list(&graph, field(&graph, entries[1], "updatedAddons"));

    // `newAddons` reuse the `addons` instances.
    assert_eq!(new_addons, [addons[1]]);
    // `deletedAddons` reuse the previous changelog's `addons` instances.
    assert_eq!(deleted, [prev_addons[1]]);
    // Updated: `@a` is top-level and lowercase (shared), `@upper` and `group/@nested` are new.
    let names = strings(&graph, &updated);
    for (value, name) in updated.iter().zip(&names) {
        match name.as_str() {
            "@a" => assert_eq!(*value, addons[0]),
            "@upper" | "group/@nested" => {
                assert!(!addons.contains(value) && !prev_addons.contains(value));
            }
            other => panic!("{other}"),
        }
    }
    // The new addons are new instances, not the previous changelog's.
    assert!(addons.iter().all(|a| !prev_addons.contains(a)));

    // Duplicate addon names are distinct instances.
    let dup = root(vec![
        dir("@x", true, vec![]),
        dir("group", false, vec![dir("@x", true, vec![])]),
    ]);
    let (_, graph) = emit(&plan_changelogs(None, None, &dup, 1, DATE, true).unwrap());
    let entries = list(&graph, field(&graph, graph.root_value().unwrap(), "list"));
    let addons = list(&graph, field(&graph, entries[0], "addons"));
    assert_eq!(strings(&graph, &addons), ["@x", "@x"]);
    assert_ne!(addons[0], addons[1]);
    assert_eq!(list(&graph, field(&graph, entries[0], "newAddons")), addons);
}

/// SHA-1 values of every leaf, in stream order.
fn leaf_sha1s(graph: &Graph) -> Vec<Value> {
    graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, _)| NodeId(u32::try_from(i).unwrap()))
        .filter(|id| {
            matches!(graph.node(*id), Ok(a3s_jserial::Node::Object(_)))
                && graph.class_name_of(*id).unwrap() == wire::SYNC_TREE_LEAF
        })
        .map(|id| graph.field(id, "sha1").unwrap())
        .collect()
}

#[test]
fn sync_sharing() {
    let tree = root(vec![
        dir("racine", false, vec![leaf("e1", "0")]),
        leaf("e2", "0"),
        leaf("e3", "0"),
        leaf("h", "abc"),
    ]);
    // All "0" hashed by this build: one instance (Java's literal).
    let fresh = [false; 4];
    let (bytes, graph) = emit(&SyncEmit::new(&tree, &fresh));
    let sha1s = leaf_sha1s(&graph);
    assert_eq!(sha1s[0], sha1s[1]);
    assert_eq!(sha1s[1], sha1s[2]);
    assert_ne!(sha1s[2], sha1s[3]);
    let decoded: SyncTreeDirectory = a3s_types::decode_stream(&bytes).unwrap();
    assert_eq!(decoded, tree);
    // Same bytes as the a3s_types encoder.
    assert!(a3s_types::encode_stream(&tree).unwrap() == bytes);
    // `racine` (root) is the literal; a nested directory named `racine` is a new string.
    let root_id = graph.root_value().unwrap();
    let nested = list(&graph, field(&graph, root_id, "list"))[0];
    assert_ne!(
        field(&graph, root_id, "name"),
        field(&graph, nested, "name")
    );

    // "0" from the SHA-1 cache: one other instance (a deserialized cache).
    let mixed = [true, false, true, false];
    let (_, graph) = emit(&SyncEmit::new(&tree, &mixed));
    let sha1s = leaf_sha1s(&graph);
    assert_eq!(sha1s[0], sha1s[2]);
    assert_ne!(sha1s[0], sha1s[1]);
}

#[test]
fn wog_changelogs_carry_over() {
    let path = wog().join("changelogs");
    let old: RawGraph = a3s_repo_files::read_a3s(&path).unwrap();
    let sync = root(vec![dir("@a", true, vec![])]);
    let unchanged = plan_changelogs(Some(&old), None, &sync, 55, DATE, false).unwrap();
    assert!(a3s_types::encode_stream(&unchanged).unwrap() == gunzip(&path));

    let fixture: Changelogs = a3s_repo_files::read_a3s(&path).unwrap();
    let changed = decoded(&plan_changelogs(Some(&old), None, &sync, 56, DATE, true).unwrap());
    assert_eq!(changed.list.len(), 10);
    assert_eq!(&changed.list[..9], &fixture.list[1..]);
    assert_eq!(changed.list[9].revision, 56);
    assert_eq!(changed.list[9].addons, ["@a"]);
    assert_eq!(changed.list[9].new_addons, ["@a"]);
    assert_eq!(changed.list[9].deleted_addons, fixture.list[9].addons);
}

struct OldEvents {
    raw: RawGraph,
}

/// Builds an old events stream:
/// * `Night`: addonNames {@a: T, GM: F, @gone: T}, dlcNames {GM: T, vn: F, WS: T, Unknown: T},
///   userconfig {cfg: T}; its description is the same instance as its name.
/// * `Day`: addonNames {@a (same instance as Night's): F}, no dlcNames field value (null),
///   userconfig with 4 entries.
/// * `Big`: 12 addons, 12 userconfig entries.
fn old_events() -> OldEvents {
    let mut b = Builder::new();
    let events_class = wire::events_desc(&mut b);
    let event_class = wire::event_desc(&mut b);
    let t = b.boolean(true);
    let f = b.boolean(false);
    let map = |b: &mut Builder, entries: Vec<(Value, Value)>| {
        let n = entries.len();
        let cap = hash_map_capacity(n);
        b.hash_map(&entries, cap, if n == 0 { 0 } else { cap * 3 / 4 })
    };
    let shared_a = b.string("@a");

    let gm_addon = b.string("GM");
    let gone = b.string("@gone");
    let night_addons = map(&mut b, vec![(shared_a, t), (gm_addon, f), (gone, t)]);
    let gm_dlc = b.string("GM");
    let vn = b.string("vn");
    let ws = b.string("WS");
    let unknown = b.string("Unknown");
    let night_dlcs = map(&mut b, vec![(gm_dlc, t), (vn, f), (ws, t), (unknown, t)]);
    let cfg = b.string("cfg");
    let night_user = map(&mut b, vec![(cfg, t)]);
    let night_name = b.string("Night");
    let night = b.simple_object(
        event_class,
        vec![night_addons, night_name, night_dlcs, night_name, night_user],
    );

    let day_addons = map(&mut b, vec![(shared_a, f)]);
    let day_user_entries: Vec<(Value, Value)> =
        (0..4).map(|i| (b.string(&format!("u{i}")), t)).collect();
    let day_user = map(&mut b, day_user_entries);
    let day_name = b.string("Day");
    let day = b.simple_object(
        event_class,
        vec![day_addons, Value::Null, Value::Null, day_name, day_user],
    );

    let big_addon_entries: Vec<(Value, Value)> =
        (0..12).map(|i| (b.string(&format!("@m{i}")), t)).collect();
    let big_addons = map(&mut b, big_addon_entries);
    let big_user_entries: Vec<(Value, Value)> =
        (0..12).map(|i| (b.string(&format!("cfg{i}")), f)).collect();
    let big_user = map(&mut b, big_user_entries);
    let empty = map(&mut b, Vec::new());
    let big_name = b.string("Big");
    let big = b.simple_object(
        event_class,
        vec![big_addons, Value::Null, empty, big_name, big_user],
    );

    let list = b.array_list(vec![night, day, big]);
    let root = b.simple_object(events_class, vec![list]);
    let bytes = a3s_jserial::write(&b.finish(root)).unwrap();
    OldEvents {
        raw: a3s_types::decode_stream(&bytes).unwrap(),
    }
}

/// `(buckets, threshold, entries)` of an emitted `HashMap`.
fn hash_map(graph: &Graph, value: Value) -> (i32, i32, Vec<(Value, Value)>) {
    let id = value.as_ref().unwrap();
    let data = graph.class_data_of(id, a3s_jserial::jdk::HASH_MAP).unwrap();
    let mut r =
        a3s_jserial::jdk::AnnotationReader::new(&data.annotations, a3s_jserial::jdk::HASH_MAP);
    let buckets = r.i32().unwrap();
    let threshold = graph.field_i32(id, "threshold").unwrap();
    (
        buckets,
        threshold,
        a3s_jserial::jdk::map(graph, value).unwrap().unwrap(),
    )
}

#[test]
fn events_carry_over() {
    let old = old_events();
    let mut addons: Vec<String> = vec!["@a".to_owned(), "GM".to_owned()];
    addons.extend((0..12).map(|i| format!("@m{i}")));
    let plan = plan_events(Some(&old.raw), &addons).unwrap();
    let (bytes, graph) = emit(&plan);
    let events: Events = a3s_types::decode_stream(&bytes).unwrap();
    assert_eq!(events.list.len(), 3);

    let night = &events.list[0];
    assert_eq!(night.name, "Night");
    assert_eq!(night.description.as_deref(), Some("Night"));
    // `GM` is both an addon name and a DLC: it stays an addon, with the DLC value.
    assert_eq!(
        night.addon_names,
        [("@a".to_owned(), true), ("GM".to_owned(), true)].into()
    );
    assert_eq!(
        night.dlc_names,
        [("WS".to_owned(), true), ("vn".to_owned(), false)].into()
    );
    assert_eq!(
        night.userconfig_folder_names,
        [("cfg".to_owned(), true)].into()
    );
    let day = &events.list[1];
    assert_eq!(day.addon_names, [("@a".to_owned(), false)].into());
    assert!(day.dlc_names.is_empty());
    assert_eq!(day.description, None);

    let items = list(&graph, field(&graph, graph.root_value().unwrap(), "list"));
    // Name and description of Night were one instance and stay one.
    assert_eq!(
        field(&graph, items[0], "name"),
        field(&graph, items[0], "description")
    );
    // The `@a` key shared by Night and Day stays one instance.
    let night_addons = hash_map(&graph, field(&graph, items[0], "addonNames")).2;
    let day_addons = hash_map(&graph, field(&graph, items[1], "addonNames")).2;
    let night_a = night_addons
        .iter()
        .find(|(k, _)| graph.opt_string(*k).unwrap() == Some("@a"))
        .unwrap()
        .0;
    assert_eq!(night_a, day_addons[0].0);

    // Capacities: sequential puts for addon/DLC maps, putAll for userconfig.
    let params = |item: Value, name: &str| {
        let (buckets, threshold, _) = hash_map(&graph, field(&graph, item, name));
        (buckets, threshold)
    };
    assert_eq!(params(items[0], "userconfigFolderNames"), (2, 1));
    assert_eq!(params(items[1], "userconfigFolderNames"), (8, 6));
    assert_eq!(params(items[2], "userconfigFolderNames"), (16, 12));
    assert_eq!(params(items[2], "addonNames"), (16, 12));
    assert_eq!(params(items[0], "dlcNames"), (16, 12));
    assert_eq!(params(items[1], "dlcNames"), (16, 0));

    // Every map is in bucket order.
    for item in &items {
        for name in ["addonNames", "dlcNames", "userconfigFolderNames"] {
            let (buckets, _, entries) = hash_map(&graph, field(&graph, *item, name));
            let indices: Vec<i32> = entries
                .iter()
                .map(|(k, _)| spread_hash(graph.opt_string(*k).unwrap().unwrap()) & (buckets - 1))
                .collect();
            assert!(indices.is_sorted(), "{name}: {indices:?}");
        }
    }

    // No old events file: an empty list.
    let none: Events = a3s_types::decode_stream(
        &a3s_types::encode_stream(&plan_events(None, &addons).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(none.list.is_empty());
}
