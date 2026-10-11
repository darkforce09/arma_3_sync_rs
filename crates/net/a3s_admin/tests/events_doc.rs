//! Events editing: the written `.a3s/events` equals Java's `writeObject` after the same edits.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_admin::events::{
    EventNameError, EventSelection, EventsDoc, EventsError, validate_event_name,
};
use a3s_jserial::jdk::{AnnotationReader, Builder, FieldSpec, HASH_MAP};
use a3s_jserial::{Graph, Node, NodeId, Value};
use a3s_types::wire;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures")
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .expect("gunzip");
    raw
}

fn stream(doc: &EventsDoc) -> Vec<u8> {
    gunzip(&doc.encode().expect("encode"))
}

fn graph(doc: &EventsDoc) -> Graph {
    a3s_jserial::read(&stream(doc)).expect("readable")
}

fn entries(value: &serde_json::Value) -> Vec<(String, bool)> {
    value
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| {
            (
                e[0].as_str().expect("key").to_owned(),
                e[1].as_bool().expect("value"),
            )
        })
        .collect()
}

fn selection(arguments: &serde_json::Value) -> EventSelection {
    EventSelection {
        addon_names: entries(&arguments["addon_names"]),
        dlc_names: entries(&arguments["dlc_names"]),
        userconfig_folder_names: entries(&arguments["userconfig_folder_names"]),
    }
}

/// The Java harness' events session: add, save, save smaller, duplicate, rename, add, remove,
/// with `.a3s/events` written after every step.
#[test]
fn java_events_session() {
    let dir = fixtures().join("java_built/events_session");
    let session: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("session.json")).expect("session"))
            .expect("json");
    let mut doc = EventsDoc::new();
    let steps = session["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 7);
    for step in steps {
        let args = &step["arguments"];
        let text = |key: &str| args[key].as_str().expect("argument").to_owned();
        match step["call"].as_str().expect("call") {
            "addEvent" => doc.add_event(&text("name"), &text("description"), &selection(args)),
            "saveEvent" => assert_eq!(doc.save_event(&text("name"), &selection(args)), 1),
            "duplicateEvent" => assert!(doc.duplicate_event(&text("event_name"))),
            "renameEvent" => assert_eq!(
                doc.rename_event(
                    &text("event_name"),
                    &text("new_event_name"),
                    &text("new_description")
                ),
                1
            ),
            "removeEvent" => assert!(doc.remove_event(&text("event_name"))),
            other => panic!("unknown call {other}"),
        }
        let expected = gunzip(
            &std::fs::read(dir.join(step["events"].as_str().expect("path"))).expect("events"),
        );
        assert_eq!(stream(&doc), expected, "{}", step["name"]);
    }
}

/// Deserializing then writing a Java file without edits is Java's round trip: the WOG file is
/// unchanged, maps filled by `putAll` get `readObject`'s table size.
#[test]
fn unedited_files_are_java_round_trips() {
    let wog = std::fs::read(fixtures().join("java_repos/wog_ftp/events")).expect("fixture");
    assert_eq!(
        stream(&EventsDoc::decode(&wog).expect("decode")),
        gunzip(&wog)
    );
    let dir = fixtures().join("java_built/events_session");
    for step in [
        "s02_save_full",
        "s04_duplicate",
        "s06_add_second",
        "s07_remove",
    ] {
        let bytes = std::fs::read(dir.join(step).join("events")).expect("fixture");
        let doc = EventsDoc::decode(&bytes).expect("decode");
        let graph = a3s_jserial::read(&gunzip(&bytes)).expect("read");
        let java = a3s_admin::upload::java_roundtrip_bytes(
            a3s_repo_files::A3sFile::Events,
            &graph,
            &a3s_admin::upload::RoundTripOverrides::default(),
        )
        .expect("round trip");
        assert_eq!(stream(&doc), gunzip(&java), "{step}");
    }
}

/// Builder for "Java-written" events streams.
struct JavaWriter {
    b: Builder,
}

impl JavaWriter {
    /// An `Event` descriptor as older versions wrote it: without `dlcNames` and
    /// `userconfigFolderNames`.
    fn old_event_desc(&mut self) -> NodeId {
        self.b.class_desc(
            wire::EVENT,
            7_456_226_002_765_813_117,
            a3s_jserial::flags::SERIALIZABLE,
            &[
                FieldSpec::object("addonNames", wire::SIG_MAP),
                FieldSpec::object("description", wire::SIG_STRING),
                FieldSpec::object("name", wire::SIG_STRING),
            ],
            None,
        )
    }
}

/// One event with 12 addons sharing the canonical `TRUE`.
fn java_written() -> Vec<u8> {
    let mut w = JavaWriter { b: Builder::new() };
    let ace = w.b.string("@ace");
    let yes = w.b.boolean(true);
    let mut addons: Vec<(Value, Value)> = vec![(ace, yes)];
    for i in 1..12 {
        let key = w.b.string(&format!("@mod{i:02}"));
        addons.push((key, yes));
    }
    let addons = w.b.hash_map(&addons, 16, 12);
    let empty_dlcs = w.b.hash_map(&[], 16, 0);
    let empty_userconfig = w.b.hash_map(&[], 16, 0);
    let name = w.b.string("Big");
    let first_class = wire::event_desc(&mut w.b);
    let first = w.b.simple_object(
        first_class,
        vec![addons, Value::Null, empty_dlcs, name, empty_userconfig],
    );
    let events_class = wire::events_desc(&mut w.b);
    let list = w.b.array_list(vec![first]);
    let root = w.b.simple_object(events_class, vec![list]);
    a3s_jserial::write(&w.b.finish(root)).expect("write")
}

/// One old event (no `dlcNames` / `userconfigFolderNames` fields) whose two keys share the
/// canonical `TRUE`.
fn old_written() -> Vec<u8> {
    let mut w = JavaWriter { b: Builder::new() };
    let class = w.old_event_desc();
    let yes = w.b.boolean(true);
    let ace = w.b.string("@ace");
    let cba = w.b.string("@cba");
    let addons = w.b.hash_map(&[(ace, yes), (cba, yes)], 16, 12);
    let name = w.b.string("Old");
    let description = w.b.string("old event");
    let event = w.b.simple_object(class, vec![addons, description, name]);
    let events_class = wire::events_desc(&mut w.b);
    let list = w.b.array_list(vec![event]);
    let root = w.b.simple_object(events_class, vec![list]);
    a3s_jserial::write(&w.b.finish(root)).expect("write")
}

fn doc_from(raw: &[u8]) -> EventsDoc {
    EventsDoc::from_graph(&a3s_jserial::read(raw).expect("read")).expect("events")
}

/// `(buckets, threshold, size)` of an event's map field.
fn map_params(graph: &Graph, event: usize, field: &str) -> Option<(i32, i32, i32)> {
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let list = a3s_jserial::jdk::list(graph, graph.field(root, "list").unwrap())
        .unwrap()
        .unwrap();
    let event = list[event].as_ref().unwrap();
    let map = graph.field(event, field).unwrap().as_ref()?;
    let data = graph.class_data_of(map, HASH_MAP).unwrap();
    let mut reader = AnnotationReader::new(&data.annotations, HASH_MAP);
    let buckets = reader.i32().unwrap();
    let size = reader.i32().unwrap();
    Some((buckets, graph.field_i32(map, "threshold").unwrap(), size))
}

fn count_strings(graph: &Graph, text: &str) -> usize {
    graph
        .nodes
        .iter()
        .filter(|n| matches!(n, Node::String(s) if s == text))
        .count()
}

fn count_booleans(graph: &Graph) -> usize {
    (0..graph.nodes.len())
        .filter(|&i| {
            let id = NodeId(u32::try_from(i).unwrap());
            graph.class_name_of(id).ok() == Some(a3s_jserial::jdk::BOOLEAN)
        })
        .count()
}

#[test]
fn deserialized_identities_and_capacities() {
    let doc = doc_from(&java_written());
    let g = graph(&doc);
    // 12 entries: `readObject` sizes the table to 16.
    assert_eq!(map_params(&g, 0, "addonNames"), Some((16, 12, 12)));
    assert_eq!(count_strings(&g, "@ace"), 1);
    // One deserialized TRUE, still one object.
    assert_eq!(count_booleans(&g), 1);

    let doc = doc_from(&old_written());
    let g = graph(&doc);
    // Null maps of the old event are materialized as new empty maps.
    assert_eq!(map_params(&g, 0, "dlcNames"), Some((16, 0, 0)));
    assert_eq!(map_params(&g, 0, "userconfigFolderNames"), Some((16, 0, 0)));
    // Two entries: 16 / 12.
    assert_eq!(map_params(&g, 0, "addonNames"), Some((16, 12, 2)));
    assert_eq!(count_booleans(&g), 1);
    // The current descriptor is written.
    let event_desc = g
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::ClassDesc(d) if d.name == wire::EVENT => Some(d.fields.len()),
            _ => None,
        })
        .unwrap();
    assert_eq!(event_desc, 5);
}

fn addons(names: &[&str]) -> EventSelection {
    EventSelection {
        addon_names: names.iter().map(|n| ((*n).to_owned(), false)).collect(),
        ..EventSelection::default()
    }
}

#[test]
fn edits_follow_java_identity_and_capacity_rules() {
    let thirteen: Vec<String> = (1..=13).map(|i| format!("@m{i:02}")).collect();
    let thirteen: Vec<&str> = thirteen.iter().map(String::as_str).collect();
    let mut doc = doc_from(&old_written());

    // Saving keeps a cleared map's table: 13 entries grow it to 32, 3 entries keep 32.
    assert_eq!(doc.save_event("Old", &addons(&thirteen)), 1);
    assert_eq!(
        map_params(&graph(&doc), 0, "addonNames"),
        Some((32, 24, 13))
    );
    assert_eq!(doc.save_event("Old", &addons(&["@a", "@b", "@c"])), 1);
    assert_eq!(map_params(&graph(&doc), 0, "addonNames"), Some((32, 24, 3)));

    // Saved values are the canonical constants, distinct from the deserialized TRUE.
    let mut doc = doc_from(&old_written());
    let mut selection = addons(&["@ace"]);
    selection.addon_names[0].1 = true;
    doc.add_event("New", "", &EventSelection::default());
    doc.save_event("New", &selection);
    let g = graph(&doc);
    assert_eq!(count_booleans(&g), 2);
    // A key saved into two events is one instance; the deserialized `@ace` is another.
    doc.add_event("Other", "", &EventSelection::default());
    doc.save_event("Other", &selection);
    assert_eq!(count_strings(&graph(&doc), "@ace"), 2);

    // New events have three empty maps.
    assert_eq!(map_params(&graph(&doc), 1, "dlcNames"), Some((16, 0, 0)));

    // Duplicates are filled with `putAll`.
    for (n, buckets, threshold) in [(1, 2, 1), (4, 8, 6), (12, 16, 12)] {
        let mut doc = EventsDoc::new();
        doc.add_event("E", "d", &EventSelection::default());
        doc.save_event("E", &addons(&thirteen[..n]));
        assert!(doc.duplicate_event("E"));
        let g = graph(&doc);
        assert_eq!(
            map_params(&g, 1, "addonNames"),
            Some((buckets, threshold, i32::try_from(n).unwrap())),
            "{n}"
        );
        assert_eq!(map_params(&g, 1, "dlcNames"), Some((16, 0, 0)));
        // Name, description and keys are shared with the source.
        assert_eq!(count_strings(&g, "d"), 1);
        assert_eq!(count_strings(&g, "@m01"), 1);
        assert_eq!(doc.events()[1].name, "E - duplicate");
    }
    assert!(!EventsDoc::new().duplicate_event("missing"));

    // Rename applies to every event of that name, with one shared name instance.
    let mut doc = EventsDoc::new();
    doc.add_event("Same", "1", &EventSelection::default());
    doc.add_event("Same", "2", &EventSelection::default());
    assert_eq!(doc.rename_event("Same", "Renamed", "desc"), 2);
    assert_eq!(count_strings(&graph(&doc), "Renamed"), 1);
    assert_eq!(doc.to_events().list.len(), 2);

    // `events()` lists each name once; remove takes the first.
    doc.add_event("Renamed", "3", &EventSelection::default());
    assert_eq!(doc.events().len(), 1);
    assert!(doc.remove_event("Renamed"));
    assert_eq!(doc.to_events().list.len(), 2);
    assert!(!doc.remove_event("missing"));
}

#[test]
fn event_name_validation() {
    let mut doc = EventsDoc::new();
    doc.add_event("Night Op", "", &EventSelection::default());
    assert_eq!(
        validate_event_name(&doc, "  Day  ", None),
        Ok("Day".to_owned())
    );
    assert_eq!(
        validate_event_name(&doc, "   ", None),
        Err(EventNameError::Empty)
    );
    assert_eq!(
        validate_event_name(&doc, " Night Op ", None),
        Err(EventNameError::Duplicate)
    );
    assert_eq!(
        validate_event_name(&doc, "Night Op", Some("Night Op")),
        Ok("Night Op".to_owned())
    );
}

#[test]
fn write_local_creates_a3s_folder_and_writes_atomically() {
    let mut doc = EventsDoc::new();
    doc.add_event("Night Op", "desc", &addons(&["@ace"]));

    assert!(matches!(
        doc.write_local(None),
        Err(EventsError::PathNotSet)
    ));
    assert!(matches!(
        doc.write_local(Some("")),
        Err(EventsError::PathNotSet)
    ));
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("missing");
    assert!(matches!(
        doc.write_local(Some(missing.to_str().unwrap())),
        Err(EventsError::PathNotFound(_))
    ));

    let path = doc
        .write_local(Some(dir.path().to_str().unwrap()))
        .expect("write");
    assert_eq!(path, dir.path().join(".a3s/events"));
    assert_eq!(gunzip(&std::fs::read(&path).unwrap()), stream(&doc));
    let names: Vec<_> = std::fs::read_dir(dir.path().join(".a3s"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["events"]);
    let typed: a3s_types::Events = a3s_repo_files::read_a3s(&path).unwrap();
    assert_eq!(typed, doc.to_events());
}
