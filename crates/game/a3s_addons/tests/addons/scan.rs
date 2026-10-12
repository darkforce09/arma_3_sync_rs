//! Discovery of mods and CDLCs on disk.

use std::fs;
use std::path::{Path, PathBuf};

use a3s_addons::priority::move_down;
use a3s_addons::{
    AVAILABLE_ROOT_NAME, ScanOptions, arma3_directory, available_addons_list,
    effective_search_directories, is_mod_directory, scan,
};
use a3s_types::constants::GameDlc;
use a3s_types::local::{Profile, TreeDirectory, TreeLeaf, TreeNode};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"").unwrap();
}

fn mkdir(path: &Path) {
    fs::create_dir_all(path).unwrap();
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn leaf(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf::new(name))
}

fn dir(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        children,
        ..TreeDirectory::new(name)
    })
}

fn root(children: Vec<TreeNode>) -> TreeDirectory {
    TreeDirectory {
        children,
        ..TreeDirectory::new(AVAILABLE_ROOT_NAME)
    }
}

fn profile(search_dirs: &[&Path], arma3_exe: Option<&Path>) -> Profile {
    let mut profile = Profile::new("test");
    profile.addon_search_directories = search_dirs.iter().map(|p| s(p)).collect();
    profile.launcher_options.arma3_exe_path = arma3_exe.map(s);
    profile
}

const CASE_SENSITIVE: ScanOptions = ScanOptions {
    case_insensitive_paths: false,
};
const CASE_INSENSITIVE: ScanOptions = ScanOptions {
    case_insensitive_paths: true,
};

#[test]
fn arma3_directory_is_the_executable_parent() {
    assert_eq!(arma3_directory(None), None);
    assert_eq!(arma3_directory(Some("")), None);
    assert_eq!(arma3_directory(Some("arma3_x64.exe")), None);
    assert_eq!(
        arma3_directory(Some("/games/Arma 3/arma3_x64.exe")),
        Some(PathBuf::from("/games/Arma 3"))
    );
}

#[test]
fn overlapping_search_directories_collapse_to_the_shorter_path() {
    let temp = tempfile::tempdir().unwrap();
    let t = temp.path();
    let mods = t.join("A").join("mods");
    let mods_x = t.join("A").join("mods_x").join("y");
    let sibling = t.join("X").join("mods");
    let sibling2 = t.join("X").join("mods2");
    for d in [&mods, &mods_x, &sibling, &sibling2] {
        mkdir(d);
    }
    let missing = t.join("missing");
    let fs_root = t.ancestors().last().unwrap().to_path_buf();

    // "<t>/A/mods" is a substring of "<t>/A/mods_x/y" although not a parent of it.
    let dirs = [
        &mods_x, &mods, &sibling, &sibling2, &missing, &fs_root, &mods,
    ]
    .map(|p| s(p));
    assert_eq!(
        effective_search_directories(&dirs, &CASE_SENSITIVE),
        [s(&mods), s(&sibling), s(&sibling2)]
    );
}

#[test]
fn windows_mode_compares_search_directories_ignoring_case() {
    let temp = tempfile::tempdir().unwrap();
    let upper = temp.path().join("W").join("Mods");
    mkdir(&upper);
    let lower = temp.path().join("W").join("mods").join("sub");
    mkdir(&lower);
    let dirs = [s(&lower), s(&upper)];
    assert_eq!(
        effective_search_directories(&dirs, &CASE_INSENSITIVE),
        [s(&upper)]
    );
    assert_eq!(effective_search_directories(&dirs, &CASE_SENSITIVE), dirs);
}

#[test]
fn a_mod_has_an_addons_folder_with_a_pbo_or_ebo_name() {
    let temp = tempfile::tempdir().unwrap();
    let t = temp.path();
    touch(&t.join("@a/addons/a.pbo"));
    touch(&t.join("@b/Addons/b.ebo"));
    touch(&t.join("@c/addons/c.pbo.zip"));
    touch(&t.join("@d/addons/D.PBO"));
    mkdir(&t.join("@e/addons"));
    touch(&t.join("@f/keys/f.pbo"));
    mkdir(&t.join("@g/addons/sub.pbo"));
    touch(&t.join("@h/addons"));

    let found: Vec<&str> = ["@a", "@b", "@c", "@d", "@e", "@f", "@g", "@h", "@missing"]
        .into_iter()
        .filter(|name| is_mod_directory(&t.join(name)))
        .collect();
    assert_eq!(found, ["@a", "@b", "@c", "@g"]);
}

#[test]
fn single_search_directory_level_is_hidden_and_mods_are_not_descended() {
    let temp = tempfile::tempdir().unwrap();
    let mods = temp.path().join("mods");
    touch(&mods.join("@a/addons/a.pbo"));
    touch(&mods.join("@a/inner/@n/addons/n.pbo"));
    touch(&mods.join("sub/@b/addons/b.pbo"));
    touch(&mods.join("sub/@b2/addons/x.pbo"));
    mkdir(&mods.join("empty/deeper"));

    let result = scan(&profile(&[&mods], None), &CASE_SENSITIVE);

    assert_eq!(
        result.tree,
        root(vec![dir("sub", vec![leaf("@b"), leaf("@b2")]), leaf("@a")])
    );
    assert_eq!(
        result.registry.keys().collect::<Vec<_>>(),
        ["@a", "@b", "@b2"]
    );
    assert_eq!(result.registry.get("@a").unwrap().path, s(&mods));
    assert_eq!(
        result.registry.get("@b").unwrap().path,
        s(&mods.join("sub"))
    );
    assert!(result.dlcs.is_empty());
    assert_eq!(
        available_addons_list(&result.tree),
        root(vec![leaf("@a"), leaf("@b"), leaf("@b2")])
    );
}

#[test]
fn search_directory_order_decides_the_keys() {
    let temp = tempfile::tempdir().unwrap();
    let one = temp.path().join("one").join("mods");
    let two = temp.path().join("two").join("mods");
    touch(&one.join("@ace/addons/ace.pbo"));
    touch(&two.join("@ace/addons/ace.pbo"));
    let mut profile = profile(&[&one, &two], None);

    let result = scan(&profile, &CASE_SENSITIVE);
    assert_eq!(result.registry.get("@ace").unwrap().path, s(&one));
    assert_eq!(result.registry.get("@ace*").unwrap().path, s(&two));
    // Both search directories are named `mods`: only the first is shown.
    assert_eq!(result.tree, root(vec![dir("mods", vec![leaf("@ace")])]));
    assert_eq!(
        available_addons_list(&result.tree),
        root(vec![leaf("@ace")])
    );

    assert!(move_down(&mut profile.addon_search_directories, 0));
    let result = scan(&profile, &CASE_SENSITIVE);
    assert_eq!(result.registry.get("@ace").unwrap().path, s(&two));
    assert_eq!(result.registry.get("@ace*").unwrap().path, s(&one));
}

#[test]
fn several_search_directories_appear_by_name() {
    let temp = tempfile::tempdir().unwrap();
    let alpha = temp.path().join("alpha");
    let beta = temp.path().join("beta");
    let none = temp.path().join("none");
    touch(&alpha.join("@a/addons/a.pbo"));
    touch(&beta.join("x/@b/addons/b.pbo"));
    mkdir(&none);

    let result = scan(&profile(&[&beta, &none, &alpha], None), &CASE_SENSITIVE);
    assert_eq!(
        result.tree,
        root(vec![
            dir("alpha", vec![leaf("@a")]),
            dir("beta", vec![dir("x", vec![leaf("@b")])]),
        ])
    );
    assert_eq!(result.registry.keys().collect::<Vec<_>>(), ["@b", "@a"]);
}

#[test]
fn arma3_folder_skips_system_folders_and_registers_dlcs_after_mods() {
    let temp = tempfile::tempdir().unwrap();
    let arma = temp.path().join("arma");
    touch(&arma.join("Addons/x.pbo"));
    touch(&arma.join("Curator/Addons/c.pbo"));
    touch(&arma.join("GM/addons/gm.pbo"));
    touch(&arma.join("!Workshop/@cba/addons/cba.pbo"));
    mkdir(&arma.join("vn"));
    let exe = arma.join("arma3_x64.exe");

    let result = scan(&profile(&[&arma], Some(&exe)), &ScanOptions::default());
    assert_eq!(
        result.tree,
        root(vec![dir("!Workshop", vec![leaf("@cba")])])
    );
    assert_eq!(result.dlcs, [GameDlc::Gm, GameDlc::Vn]);
    assert_eq!(
        result.registry.keys().collect::<Vec<_>>(),
        ["@cba", "gm", "vn"]
    );
    let gm = result.registry.get("GM").unwrap();
    assert_eq!((gm.key.as_str(), gm.path.clone()), ("GM", s(&arma)));

    // A mod folder named `GM` found first keeps the plain key.
    let other = temp.path().join("other");
    touch(&other.join("GM/addons/gm.pbo"));
    let result = scan(
        &profile(&[&other, &arma], Some(&exe)),
        &ScanOptions::default(),
    );
    assert_eq!(result.registry.get("GM").unwrap().path, s(&other));
    let dlc = result.registry.get("GM*").unwrap();
    assert_eq!((dlc.name.as_str(), dlc.path.clone()), ("GM", s(&arma)));
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_followed_but_loops_are_cut() {
    let temp = tempfile::tempdir().unwrap();
    let mods = temp.path().join("mods");
    let elsewhere = temp.path().join("elsewhere");
    touch(&mods.join("@a/addons/a.pbo"));
    touch(&elsewhere.join("@z/addons/z.pbo"));
    std::os::unix::fs::symlink(&mods, mods.join("loop")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, mods.join("link")).unwrap();

    let result = scan(&profile(&[&mods], None), &CASE_SENSITIVE);
    assert_eq!(
        result.tree,
        root(vec![dir("link", vec![leaf("@z")]), leaf("@a")])
    );
    assert_eq!(result.registry.keys().collect::<Vec<_>>(), ["@a", "@z"]);
}
