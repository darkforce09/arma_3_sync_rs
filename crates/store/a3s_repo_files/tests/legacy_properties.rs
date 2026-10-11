//! `java.util.Properties` parsing and the Java Workshop metadata file (A10).

use a3s_repo_files::legacy::properties::{PropertiesError, parse};
use a3s_repo_files::legacy::read_workshop_file;
use a3s_repo_files::local::WorkshopEntry;

fn pairs(input: &[u8]) -> Vec<(String, String)> {
    parse(input).unwrap()
}

fn p(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

#[test]
fn a10_parses_like_java_properties_load() {
    let input: &[u8] = b"#Arma3Sync known Workshop metadata\n\
#Sat Oct 11 12:00:00 CEST 2026\n\
mod.QGFjZQ.key=@ace\n\
mod.QGFjZQ.name=ACE\r\n\
mod.QGFjZQ.publishedid=463939057\r\
key\\:with\\=seps\\ and\\ space = value \\u00E9 \\t tab\n\
latin1=\xE9t\xE9\n\
continued = first \\\n    second \\\r\n  third\n\
! bang comment\n\
\x20\x20 # indented comment\n\
\n\
\x20\t \n\
dup=1\n\
dup=2\n\
odd\\\\=even\n\
lonely\n\
spaced   :  colon\n\
x=trailing\\";
    assert_eq!(
        pairs(input),
        vec![
            p("mod.QGFjZQ.key", "@ace"),
            p("mod.QGFjZQ.name", "ACE"),
            p("mod.QGFjZQ.publishedid", "463939057"),
            p("key:with=seps and space", "value \u{e9} \t tab"),
            p("latin1", "\u{e9}t\u{e9}"),
            p("continued", "first second third"),
            p("dup", "1"),
            p("dup", "2"),
            p("odd\\", "even"),
            p("lonely", ""),
            p("spaced", "colon"),
            p("x", "trailing"),
        ]
    );
}

#[test]
fn a10_continuation_lines_are_not_comments() {
    assert_eq!(pairs(b"a=1\\\n#2\n"), vec![p("a", "1#2")]);
    // Escapes other than t, n, r, f, u yield the character itself.
    assert_eq!(pairs(b"a=\\q\\#\\n"), vec![p("a", "q#\n")]);
}

#[test]
fn a10_malformed_unicode_escape_is_an_error() {
    assert_eq!(
        parse(b"a=\\u12"),
        Err(PropertiesError::MalformedUnicodeEscape)
    );
    assert_eq!(
        parse(b"a=\\u12zz"),
        Err(PropertiesError::MalformedUnicodeEscape)
    );
}

fn entry(key: &str, id: &str, name: &str) -> WorkshopEntry {
    WorkshopEntry {
        addon_key: key.to_owned(),
        published_id: id.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn a10_read_workshop_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known-workshop-mods.properties");
    std::fs::write(
        &path,
        "#Arma3Sync known Workshop metadata\n\
         #Sat Oct 11 12:00:00 CEST 2026\n\
         mod.QGFjZQ.key=@ace\n\
         mod.QGFjZQ.name=ACE\n\
         mod.QGFjZQ.publishedid=463939057\n\
         mod.QGNiYQ.key=@cba\n\
         mod.QGNiYQ.name=\\  \n\
         mod.QGNiYQ.publishedid=450814997\n\
         mod.not*b64.key=@odd\n\
         mod.not*b64.name=Odd Mod \n\
         mod.not*b64.publishedid=12\n\
         mod.bad*seg.name=No key\n\
         mod.bad*seg.publishedid=13\n\
         mod.QGlk.publishedid=14\n",
    )
    .unwrap();
    let file = read_workshop_file(&path);
    assert_eq!(
        file.value,
        vec![
            entry("@ace", "463939057", "ACE"),
            entry("@odd", "12", "Odd Mod"),
        ]
    );
    // `bad*seg` has neither a decodable segment nor a `.key`.
    assert_eq!(file.warnings.len(), 1, "{:?}", file.warnings);
    assert!(file.warnings[0].starts_with("known-workshop-mods.properties: "));
}

#[test]
fn a10_unreadable_workshop_file_is_empty_with_one_warning() {
    let dir = tempfile::tempdir().unwrap();
    let missing = read_workshop_file(&dir.path().join("missing.properties"));
    assert!(missing.value.is_empty());
    assert_eq!(missing.warnings.len(), 1);

    let malformed = dir.path().join("bad.properties");
    std::fs::write(&malformed, "mod.QGFjZQ.name=\\u12\n").unwrap();
    let file = read_workshop_file(&malformed);
    assert!(file.value.is_empty());
    assert_eq!(file.warnings.len(), 1);
}
