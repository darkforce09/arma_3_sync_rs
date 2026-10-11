//! Removing `class ModLauncherList` from `Arma3.cfg`.

use a3s_launch::cfg::{ARMA3_CFG, erase_arma3_cfg_mod_launcher_list, strip_mod_launcher_list};

#[test]
fn strips_the_list_and_normalises_line_endings() {
    let input = b"language=\"English\";\r\nclass ModLauncherList\r\n{\r\n\tclass Mod1\r\n\t{\r\n\t\tdir=\"@CBA_A3\";\r\n\t};\r\n};\r\nfoo=1;\r\nversion=1";
    assert_eq!(
        strip_mod_launcher_list(input).unwrap(),
        b"language=\"English\";\nfoo=1;\nversion=1\n"
    );
}

#[test]
fn every_list_goes() {
    let input = b"a\nclass ModLauncherList\n{\n};\nb\nclass ModLauncherList\n{\n\t};\n};\nc\n";
    assert_eq!(strip_mod_launcher_list(input).unwrap(), b"a\nb\nc\n");
}

#[test]
fn unterminated_list_drops_the_rest() {
    let input = b"a\nclass ModLauncherList\n{\n\tclass Mod1\nb\n";
    assert_eq!(strip_mod_launcher_list(input).unwrap(), b"a\n");
}

#[test]
fn only_exact_lines_match() {
    assert_eq!(
        strip_mod_launcher_list(b"a\n class ModLauncherList\n{\n};\n"),
        None
    );
    assert_eq!(strip_mod_launcher_list(b""), None);
}

#[test]
fn lone_carriage_return_ends_a_line() {
    let input = b"a\rclass ModLauncherList\r{\r};\rb";
    assert_eq!(strip_mod_launcher_list(input).unwrap(), b"a\nb\n");
}

#[test]
fn bytes_outside_the_list_survive() {
    let input = b"name=\"\xff\xfe na\xc3\xafve\";\nclass ModLauncherList\n};\n";
    assert_eq!(
        strip_mod_launcher_list(input).unwrap(),
        b"name=\"\xff\xfe na\xc3\xafve\";\n"
    );
}

#[test]
fn erase_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(ARMA3_CFG);
    assert!(!erase_arma3_cfg_mod_launcher_list(dir.path()).unwrap());

    std::fs::write(&path, b"a=1;\r\n").unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(!erase_arma3_cfg_mod_launcher_list(dir.path()).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"a=1;\r\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );

    std::fs::write(&path, b"a=1;\r\nclass ModLauncherList\r\n{\r\n};\r\n").unwrap();
    assert!(erase_arma3_cfg_mod_launcher_list(dir.path()).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"a=1;\n");
}
