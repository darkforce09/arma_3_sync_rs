//! Reading known streams and round-tripping built graphs.

use a3s_jserial::jdk::{self, Builder, FieldSpec};
use a3s_jserial::{Content, FieldType, Node, Value, flags, read, write};

/// The example from the Java Object Serialization Specification, section 6.4:
///
/// ```java
/// class List implements java.io.Serializable { int value; List next; }
/// // list1 = {17, list2}, list2 = {19, null}
/// out.writeObject(list1); out.writeObject(list2);
/// ```
const SPEC_EXAMPLE: &[u8] = &[
    0xac, 0xed, 0x00, 0x05, 0x73, 0x72, 0x00, 0x04, 0x4c, 0x69, 0x73, 0x74, 0x69, 0xc8, 0x8a, 0x15,
    0x40, 0x16, 0xae, 0x68, 0x02, 0x00, 0x02, 0x49, 0x00, 0x05, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x4c,
    0x00, 0x04, 0x6e, 0x65, 0x78, 0x74, 0x74, 0x00, 0x06, 0x4c, 0x4c, 0x69, 0x73, 0x74, 0x3b, 0x78,
    0x70, 0x00, 0x00, 0x00, 0x11, 0x73, 0x71, 0x00, 0x7e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x13, 0x70,
    0x71, 0x00, 0x7e, 0x00, 0x03,
];

#[test]
fn spec_example_reads() {
    let graph = read(SPEC_EXAMPLE).unwrap();
    assert_eq!(graph.roots.len(), 2);

    let Content::Value(Value::Ref(list1)) = graph.roots[0] else {
        panic!("expected object")
    };
    let Content::Value(Value::Ref(list2_again)) = graph.roots[1] else {
        panic!("expected object")
    };

    assert_eq!(graph.class_name_of(list1).unwrap(), "List");
    let desc = graph
        .class_desc(graph.object(list1).unwrap().class)
        .unwrap();
    assert_eq!(desc.serial_version_uid, 0x69c8_8a15_4016_ae68);
    assert_eq!(desc.flags, flags::SERIALIZABLE);

    assert_eq!(graph.field_i32(list1, "value").unwrap(), 17);
    let Value::Ref(list2) = graph.field(list1, "next").unwrap() else {
        panic!("expected next")
    };
    assert_eq!(graph.field_i32(list2, "value").unwrap(), 19);
    assert_eq!(graph.field(list2, "next").unwrap(), Value::Null);

    // The second top-level write is a back-reference to the same instance.
    assert_eq!(list2, list2_again);
}

#[test]
fn spec_example_rewrites_identically() {
    let graph = read(SPEC_EXAMPLE).unwrap();
    assert_eq!(write(&graph).unwrap(), SPEC_EXAMPLE);
}

/// Builds a small tree with parent back-references, collections, enums, dates and booleans.
fn sample_graph() -> a3s_jserial::Graph {
    let mut b = Builder::new();
    let node_class = b.class_desc(
        "test.Node",
        42,
        flags::SERIALIZABLE,
        &[
            FieldSpec::prim(FieldType::Boolean, "flag"),
            FieldSpec::prim(FieldType::Long, "size"),
            FieldSpec::object("children", "Ljava/util/List;"),
            FieldSpec::object("kind", "Ltest/Kind;"),
            FieldSpec::object("name", "Ljava/lang/String;"),
            FieldSpec::object("parent", "Ltest/Node;"),
        ],
        None,
    );
    let root = b.reserve_object(node_class);
    let child = b.reserve_object(node_class);

    let child_name = b.string("child ✓ \0 😀");
    let kind = b.enum_value("test.Kind", "LEAF");
    let empty = b.array_list(vec![]);
    b.set_object_data(
        child,
        vec![a3s_jserial::ClassData {
            values: vec![
                Value::Boolean(true),
                Value::Long(-5),
                empty,
                kind,
                child_name,
                Value::Ref(root),
            ],
            annotations: vec![],
        }],
    );

    let children = b.array_list(vec![Value::Ref(child)]);
    let kind = b.enum_value("test.Kind", "LEAF");
    let name = b.interned("racine");
    b.set_object_data(
        root,
        vec![a3s_jserial::ClassData {
            values: vec![
                Value::Boolean(false),
                Value::Long(1 << 40),
                children,
                kind,
                name,
                Value::Null,
            ],
            annotations: vec![],
        }],
    );

    // Wrap everything in a list together with JDK types.
    let t = b.boolean(true);
    let f = b.boolean(false);
    let map = b.hash_map_str(&[
        ("@ace".to_owned(), t),
        ("@cba_a3".to_owned(), f),
        ("@rhs".to_owned(), t),
    ]);
    let set = b.hash_set_str(&["a".to_owned(), "b/c".to_owned()]);
    let date = b.date(1_700_000_000_123);
    let list = b.array_list(vec![Value::Ref(root), map, set, date, Value::Null]);
    b.finish(list)
}

#[test]
fn built_graph_round_trips() {
    let graph = sample_graph();
    let bytes = write(&graph).unwrap();
    let reread = read(&bytes).unwrap();
    // Re-writing the parsed stream must give identical bytes.
    assert_eq!(write(&reread).unwrap(), bytes);

    let elements = jdk::list(&reread, reread.root_value().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(elements.len(), 5);

    let Value::Ref(root) = elements[0] else {
        panic!()
    };
    assert_eq!(
        reread.field_string(root, "name").unwrap().unwrap(),
        "racine"
    );
    assert_eq!(reread.field_i64(root, "size").unwrap(), 1 << 40);
    assert_eq!(
        reread
            .enum_name(reread.field(root, "kind").unwrap())
            .unwrap(),
        Some("LEAF")
    );
    let children = jdk::list(&reread, reread.field(root, "children").unwrap())
        .unwrap()
        .unwrap();
    let Value::Ref(child) = children[0] else {
        panic!()
    };
    assert_eq!(reread.field(child, "parent").unwrap(), Value::Ref(root));
    assert_eq!(
        reread.field_string(child, "name").unwrap().unwrap(),
        "child ✓ \0 😀"
    );

    let map = jdk::map(&reread, elements[1]).unwrap().unwrap();
    let mut decoded: Vec<(String, bool)> = map
        .iter()
        .map(|(k, v)| {
            (
                reread.opt_string(*k).unwrap().unwrap().to_owned(),
                jdk::boolean(&reread, *v).unwrap().unwrap(),
            )
        })
        .collect();
    decoded.sort();
    assert_eq!(
        decoded,
        [
            ("@ace".to_owned(), true),
            ("@cba_a3".to_owned(), false),
            ("@rhs".to_owned(), true)
        ]
    );
    // Boolean.TRUE is a shared instance.
    let trues: Vec<Value> = map
        .iter()
        .filter(|(_, v)| jdk::boolean(&reread, *v).unwrap() == Some(true))
        .map(|(_, v)| *v)
        .collect();
    assert_eq!(trues.len(), 2);
    assert_eq!(trues[0], trues[1]);

    let set = jdk::set(&reread, elements[2]).unwrap().unwrap();
    assert_eq!(set.len(), 2);
    assert_eq!(
        jdk::date(&reread, elements[3]).unwrap(),
        Some(1_700_000_000_123)
    );
    assert_eq!(elements[4], Value::Null);
}

#[test]
fn enum_constants_are_shared() {
    let graph = sample_graph();
    let enums = graph
        .nodes
        .iter()
        .filter(|n| matches!(n, Node::Enum(_)))
        .count();
    assert_eq!(enums, 1);
}

#[test]
fn truncated_streams_fail_cleanly() {
    let bytes = write(&sample_graph()).unwrap();
    for len in 0..bytes.len() {
        assert!(read(&bytes[..len]).is_err() || len == 4, "len {len}");
    }
}

#[test]
fn garbage_does_not_panic() {
    let bytes = write(&sample_graph()).unwrap();
    for i in 4..bytes.len() {
        let mut corrupted = bytes.clone();
        corrupted[i] ^= 0x5a;
        let _ignored = read(&corrupted);
    }
}
