//! Decodes the `.a3s` metadata files in a directory and prints a summary.
//!
//! ```sh
//! cargo run -p a3s_repo_files --example inspect_a3s -- path/to/.a3s
//! ```

#![expect(clippy::print_stdout)]

use std::path::PathBuf;

use a3s_repo_files::{A3sFile, read_a3s};
use a3s_types::{AutoConfig, Changelogs, Events, ServerInfo, SyncTreeDirectory};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("usage: inspect_a3s <dir>")?);
    let path = |f: A3sFile| dir.join(f.file_name());

    let info: ServerInfo = read_a3s(&path(A3sFile::ServerInfo))?;
    println!("serverinfo: {info:#?}");

    let auto: AutoConfig = read_a3s(&path(A3sFile::AutoConfig))?;
    println!("autoconfig: {auto:#?}");

    let changelogs: Changelogs = read_a3s(&path(A3sFile::Changelogs))?;
    for c in &changelogs.list {
        println!(
            "changelog rev {} updated={} new={:?} updated={:?} deleted={:?} addons={}",
            c.revision,
            c.content_updated,
            c.new_addons,
            c.updated_addons,
            c.deleted_addons,
            c.addons.len()
        );
    }

    let events: Events = read_a3s(&path(A3sFile::Events))?;
    println!("events: {events:#?}");

    let sync: SyncTreeDirectory = read_a3s(&path(A3sFile::Sync))?;
    let total: i64 = {
        let mut sum = 0;
        sync.walk(&mut |_, node| {
            if let a3s_types::SyncTreeNode::Leaf(l) = node {
                sum += l.size;
            }
        });
        sum
    };
    println!(
        "sync: root={:?} top-level={:?} leaves={} total-size={total}",
        sync.name,
        sync.children.iter().map(|c| c.name()).collect::<Vec<_>>(),
        sync.leaf_count(),
    );
    let mut shown = 0;
    sync.walk(&mut |path, node| {
        if let a3s_types::SyncTreeNode::Leaf(l) = node
            && shown < 5
        {
            shown += 1;
            println!(
                "  {path} size={} sha1={:?} compressed={}",
                l.size, l.sha1, l.compressed
            );
        }
    });

    // Re-encode and compare decompressed bytes with the original.
    for file in A3sFile::ALL {
        let original = std::fs::read(path(file))?;
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&original[..]), &mut raw)?;
        let reencoded = match file {
            A3sFile::Sync => a3s_types::encode_stream(&sync)?,
            A3sFile::ServerInfo => a3s_types::encode_stream(&info)?,
            A3sFile::Changelogs => a3s_types::encode_stream(&changelogs)?,
            A3sFile::AutoConfig => a3s_types::encode_stream(&auto)?,
            A3sFile::Events => a3s_types::encode_stream(&events)?,
        };
        let generic = a3s_jserial::write(&a3s_jserial::read(&raw)?)?;
        println!(
            "{:<11} raw={:>7} generic-rewrite-identical={} typed-reencode-identical={}",
            file.file_name(),
            raw.len(),
            generic == raw,
            reencoded == raw
        );
    }
    Ok(())
}
