//! Event presets in the active profile.

use std::collections::BTreeMap;

use a3s_types::Event;
use a3s_types::constants::ModsetType;
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};
use a3s_viewer_context::profiles;

use crate::support::Fixture;

fn group<'t>(root: &'t TreeDirectory, name: &str) -> &'t TreeDirectory {
    root.directories()
        .find(|group| group.name == name)
        .expect("group")
}

fn selected(group: &TreeDirectory, leaf: &str) -> bool {
    group.children.iter().any(|node| match node {
        TreeNode::Leaf(found) => found.name == leaf && found.selected,
        TreeNode::Directory(_) => false,
    })
}

#[test]
fn an_event_preset_enables_exactly_the_required_mods_of_the_event() {
    let fixture = Fixture::new(Vec::new());
    let event = Event {
        name: "Operation".to_owned(),
        addon_names: BTreeMap::from([("@ace".to_owned(), false), ("@maps".to_owned(), true)]),
        ..Event::default()
    };
    let changed = fixture
        .ctx
        .mutate(None, |state| {
            state.client.create_profile("P")?;
            state.client.set_active_profile("P")?;
            state.client.update_active_profile(|profile| {
                let mut core = TreeDirectory::new("Core");
                core.selected = true;
                core.add_child(TreeNode::Leaf(TreeLeaf {
                    selected: true,
                    ..TreeLeaf::new("@cba")
                }));
                profile.tree.add_child(TreeNode::Directory(core));
                true
            })?;
            profiles::use_event_preset(state, "Alpha", &event)
        })
        .expect("preset");
    assert!(changed);
    {
        let state = fixture.ctx.read();
        let tree = &state.client.active_profile().expect("profile").tree;
        assert!(!selected(group(tree, "Core"), "@cba"));
        let operation = group(tree, "Operation");
        assert_eq!(operation.modset_type, Some(ModsetType::Event));
        assert_eq!(operation.modset_repository_name.as_deref(), Some("Alpha"));
        assert!(selected(operation, "@ace"));
        assert!(!selected(operation, "@maps"));
    }

    let again = fixture
        .ctx
        .mutate(None, |state| {
            profiles::use_event_preset(state, "Alpha", &event)
        })
        .expect("preset");
    assert!(!again, "a preset already in use changes nothing");
}

#[test]
fn a_group_preset_enables_exactly_that_group() {
    let fixture = Fixture::new(Vec::new());
    let changed = fixture
        .ctx
        .mutate(None, |state| {
            state.client.create_profile("P")?;
            state.client.set_active_profile("P")?;
            state.client.update_active_profile(|profile| {
                for (name, selected) in [("Core", true), ("Server", false)] {
                    let mut group = TreeDirectory::new(name);
                    group.selected = selected;
                    group.add_child(TreeNode::Leaf(TreeLeaf {
                        selected,
                        ..TreeLeaf::new(format!("@{name}"))
                    }));
                    profile.tree.add_child(TreeNode::Directory(group));
                }
                profile.tree.add_child(TreeNode::Leaf(TreeLeaf {
                    selected: true,
                    ..TreeLeaf::new("@loose")
                }));
                true
            })?;
            profiles::use_group_preset(state, "Server")
        })
        .expect("preset");
    assert!(changed);
    let state = fixture.ctx.read();
    let tree = &state.client.active_profile().expect("profile").tree;
    assert!(selected(group(tree, "Server"), "@Server"));
    assert!(!selected(group(tree, "Core"), "@Core"));
    assert!(!selected(tree, "@loose"));

    drop(state);
    let missing = fixture
        .ctx
        .mutate(None, |state| profiles::use_group_preset(state, "Gone"))
        .expect("preset");
    assert!(!missing, "a preset whose group is gone changes nothing");
}
