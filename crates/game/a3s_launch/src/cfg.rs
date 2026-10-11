//! Removing the Arma 3 Launcher's mod list from `Arma3.cfg`, so the game loads only the `-mod=`
//! parameters.

use std::io;
use std::path::Path;

/// The game's configuration file in the Arma 3 user folder.
pub const ARMA3_CFG: &str = "Arma3.cfg";

const LIST_START: &[u8] = b"class ModLauncherList";
const LIST_END: &[u8] = b"};";

/// `content` without its `class ModLauncherList … };` blocks, or `None` if it has none.
///
/// Lines end at `\n`, `\r\n` or `\r` (`BufferedReader.readLine`). A line exactly
/// `class ModLauncherList` starts a block, which runs to the first line exactly `};` (indented
/// `\t};` lines do not end it); without one the block runs to the end. The kept lines are
/// written with `\n` endings, a final one included. Bytes are kept as they are (Java decodes and
/// re-encodes, replacing invalid UTF-8).
// Mirrors service/LaunchService.java:170-204
pub fn strip_mod_launcher_list(content: &[u8]) -> Option<Vec<u8>> {
    let mut kept = Vec::with_capacity(content.len());
    let mut record = true;
    let mut found = false;
    for line in lines(content) {
        if line == LIST_START {
            record = false;
            found = true;
        } else if !record && line == LIST_END {
            record = true;
            continue;
        }
        if record {
            kept.extend_from_slice(line);
            kept.push(b'\n');
        }
    }
    found.then_some(kept)
}

/// The lines of `content`, like `BufferedReader.readLine`.
fn lines(content: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < content.len() {
        match content[i] {
            b'\n' => {
                lines.push(&content[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&content[start..i]);
                i += 1;
                if content.get(i) == Some(&b'\n') {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    lines
}

/// Strips the mod list from `<arma3_documents_dir>/Arma3.cfg` in place.
///
/// See [`strip_mod_launcher_list`]. `Ok(false)` when the file is missing or has no list (it is
/// then not touched), `Ok(true)` when it was rewritten.
// Mirrors service/LaunchService.java:157-208
pub fn erase_arma3_cfg_mod_launcher_list(arma3_documents_dir: &Path) -> io::Result<bool> {
    let path = arma3_documents_dir.join(ARMA3_CFG);
    let content = match std::fs::read(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    match strip_mod_launcher_list(&content) {
        Some(stripped) => {
            std::fs::write(&path, stripped)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
