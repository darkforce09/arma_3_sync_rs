//! The Arma 3 profile names next to the Arma 3 user folder.

use std::fs;

use a3s_platform::folders::game_profile_names;

#[test]
fn game_profiles_come_from_the_other_profiles_folder() {
    let documents = tempfile::tempdir().expect("temp dir");
    let other = documents.path().join("Arma 3 - Other Profiles");
    for name in ["Sam", "John%20Doe", "Mr%2e%20X"] {
        fs::create_dir_all(other.join(name)).expect("profile folder");
    }
    fs::create_dir_all(documents.path().join("Arma 3")).expect("user folder");
    fs::create_dir_all(documents.path().join("Other folder/Ignored")).expect("other folder");

    let names = game_profile_names(&documents.path().join("Arma 3"));
    assert_eq!(names, ["John Doe", "Mr. X", "Sam"]);
}

#[test]
fn no_other_profiles_folder_gives_no_names() {
    let documents = tempfile::tempdir().expect("temp dir");
    assert!(game_profile_names(&documents.path().join("Arma 3")).is_empty());
}
