//! Copying the `userconfig` folder into the Arma 3 folder.

use std::path::Path;

use a3s_sync::PlanTree;
use a3s_sync::pipeline::{
    UserconfigAction, UserconfigCopy, UserconfigOutcome, copy_userconfig, plan_userconfig_copy,
};
use a3s_sync::plan::{NoAddons, assign_destinations};
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

fn tree(dest: &Path) -> PlanTree {
    let mut root = SyncTreeDirectory::new_root();
    root.children = vec![SyncTreeNode::Directory(SyncTreeDirectory {
        name: "UserConfig".to_owned(),
        children: vec![SyncTreeNode::Leaf(SyncTreeLeaf {
            name: "ace_settings.hpp".to_owned(),
            ..Default::default()
        })],
        ..Default::default()
    })];
    let mut tree = PlanTree::from_sync_tree(&root);
    assign_destinations(&mut tree, dest, false, &NoAddons).expect("names");
    tree
}

#[test]
fn plan_userconfig_copy_cases() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    let arma3 = base.path().join("arma3");
    let tree = tree(&dest);
    let exe = arma3.join("arma3_x64.exe");

    assert_eq!(
        plan_userconfig_copy(&tree, None, &dest, Some(&exe)),
        UserconfigAction::NotNeeded
    );
    assert_eq!(
        plan_userconfig_copy(&tree, Some("UserConfig"), &dest, None),
        UserconfigAction::NoArma3Directory
    );
    assert_eq!(
        plan_userconfig_copy(&tree, Some("UserConfig"), &dest, Some(Path::new(""))),
        UserconfigAction::NoArma3Directory
    );
    assert_eq!(
        plan_userconfig_copy(
            &tree,
            Some("UserConfig"),
            &dest,
            Some(&dest.join("arma3.exe"))
        ),
        UserconfigAction::SameFolder
    );
    assert_eq!(
        plan_userconfig_copy(&tree, Some("UserConfig"), &dest, Some(&exe)),
        UserconfigAction::Copy(UserconfigCopy {
            source: dest.join("UserConfig"),
            target: arma3.join("UserConfig"),
        })
    );
}

#[test]
fn copy_userconfig_outcomes() {
    let base = tempfile::tempdir().expect("tempdir");
    let source = base.path().join("dest/UserConfig");
    let arma3 = base.path().join("arma3");
    let copy = UserconfigCopy {
        source: source.clone(),
        target: arma3.join("UserConfig"),
    };

    // Missing source: nothing is created.
    std::fs::create_dir_all(&arma3).expect("mkdir");
    assert!(matches!(
        copy_userconfig(&copy),
        UserconfigOutcome::SourceMissing(_)
    ));
    assert!(!copy.target.exists());

    std::fs::create_dir_all(source.join("ace")).expect("mkdir");
    std::fs::write(source.join("ace/ace_settings.hpp"), b"settings").expect("write");
    std::fs::create_dir_all(&copy.target).expect("mkdir");
    std::fs::write(copy.target.join("mine.hpp"), b"keep").expect("write");
    let outcome = copy_userconfig(&copy);
    assert!(
        matches!(outcome, UserconfigOutcome::Copied(_)),
        "{outcome:?}"
    );
    assert!(outcome.message().is_some());
    assert_eq!(
        std::fs::read(copy.target.join("ace/ace_settings.hpp")).expect("copied"),
        b"settings"
    );
    assert!(copy.target.join("mine.hpp").exists());

    // The Arma 3 folder is missing: the target cannot be created one level deep.
    let unreachable = UserconfigCopy {
        source,
        target: base.path().join("missing/arma3/UserConfig"),
    };
    assert!(matches!(
        copy_userconfig(&unreachable),
        UserconfigOutcome::CreateFailed(_)
    ));
    assert_eq!(UserconfigOutcome::Declined.message(), None);

    // A target that is a link is not written through.
    #[cfg(unix)]
    {
        let elsewhere = base.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).expect("mkdir");
        let linked = UserconfigCopy {
            source: copy.source.clone(),
            target: base.path().join("arma3/LinkedConfig"),
        };
        std::os::unix::fs::symlink(&elsewhere, &linked.target).expect("symlink");
        assert!(matches!(
            copy_userconfig(&linked),
            UserconfigOutcome::CopyFailed(_)
        ));
        assert!(
            std::fs::read_dir(&elsewhere)
                .expect("read")
                .next()
                .is_none()
        );
    }
}
