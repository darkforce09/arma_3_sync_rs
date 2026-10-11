//! Encoding, validation and writing of the five `.a3s` files.

use std::path::Path;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{ClassData, Graph, JavaFilterLimits, NodeId, Value};
use a3s_repo_files::{A3sFile, write_a3s_stream};
use a3s_types::{JavaCodec, SYNC_ROOT_NAME, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode, wire};

use super::BuildError;
use super::hash::is_empty_file_sha1;

/// Largest decompressed stream Rust's own `.a3s` reader accepts (`a3s_repo_files`, 256 MiB).
const MAX_STREAM_BYTES: u64 = 256 * 1024 * 1024;

/// The sync tree as a build writes it.
///
/// Like [`SyncTreeDirectory`]'s own encoder, except for the empty-file SHA-1 `"0"`: leaves hashed
/// by this build share Java's string literal, while leaves whose `"0"` came from the SHA-1 cache
/// share the one instance a deserialized cache holds (Java reloads the cache with the repository
/// in a new session). A same-session rebuild that adds empty files is the one case where Java
/// shares all of them.
pub struct SyncEmit<'a> {
    tree: &'a SyncTreeDirectory,

    /// Per deep-search leaf index: the `"0"` comes from the cache.
    cached_zero: &'a [bool],
}

impl<'a> SyncEmit<'a> {
    /// `cached_zero[i]`: the i-th leaf (deep-search order) took its `"0"` from the SHA-1 cache.
    pub fn new(tree: &'a SyncTreeDirectory, cached_zero: &'a [bool]) -> Self {
        Self { tree, cached_zero }
    }

    fn encode_dir(
        &self,
        b: &mut Builder,
        dir: &SyncTreeDirectory,
        parent: Option<NodeId>,
        state: &mut EmitState,
    ) -> Value {
        let class = wire::sync_tree_directory_desc(b);
        let id = b.reserve_object(class);
        let children = dir
            .children
            .iter()
            .map(|child| match child {
                SyncTreeNode::Directory(d) => self.encode_dir(b, d, Some(id), state),
                SyncTreeNode::Leaf(l) => self.encode_leaf(b, l, id, state),
            })
            .collect();
        let list = b.array_list(children);
        let name = if parent.is_none() && dir.name == SYNC_ROOT_NAME {
            // `SyncTreeDirectory.RACINE`, a string literal.
            b.interned(SYNC_ROOT_NAME)
        } else {
            b.string(&dir.name)
        };
        b.set_object_data(
            id,
            vec![ClassData {
                values: vec![
                    Value::Boolean(dir.deleted),
                    Value::Boolean(dir.hidden),
                    Value::Boolean(dir.mark_as_addon),
                    Value::Boolean(dir.updated),
                    list,
                    name,
                    parent.map_or(Value::Null, Value::Ref),
                ],
                annotations: Vec::new(),
            }],
        );
        Value::Ref(id)
    }

    fn encode_leaf(
        &self,
        b: &mut Builder,
        leaf: &SyncTreeLeaf,
        parent: NodeId,
        state: &mut EmitState,
    ) -> Value {
        let index = state.next_leaf;
        state.next_leaf += 1;
        let class = wire::sync_tree_leaf_desc(b);
        let name = b.string(&leaf.name);
        let sha1 = if is_empty_file_sha1(leaf.sha1.as_deref()) {
            if self.cached_zero.get(index).copied().unwrap_or(false) {
                *state.cached_zero.get_or_insert_with(|| b.string("0"))
            } else {
                // Mirrors dao/FileAccessMethods.java:238-240
                b.interned("0")
            }
        } else {
            b.opt_string(leaf.sha1.as_deref())
        };
        b.simple_object(
            class,
            vec![
                Value::Boolean(leaf.compressed),
                Value::Long(leaf.compressed_size),
                Value::Boolean(leaf.deleted),
                Value::Long(leaf.size),
                Value::Boolean(leaf.updated),
                name,
                Value::Ref(parent),
                sha1,
            ],
        )
    }
}

#[derive(Default)]
struct EmitState {
    next_leaf: usize,
    cached_zero: Option<Value>,
}

impl JavaCodec for SyncEmit<'_> {
    fn decode(_graph: &Graph, _value: Value) -> a3s_types::Result<Self> {
        Err(a3s_jserial::Error::Unsupported("build output is write-only").into())
    }

    fn encode(&self, b: &mut Builder) -> Value {
        self.encode_dir(b, self.tree, None, &mut EmitState::default())
    }
}

/// Encodes `value` and checks that both Java and Rust readers accept the stream.
///
/// Java's check is exact: [`a3s_jserial::java_filter_stats`] replays the JDK 25 serial filter
/// counters (`depth`, `totalObjectRefs`, `streamBytes`) at every point `ObjectInputStream` calls
/// the filter, and `A3SFilesAccessor`'s limits are applied to them.
///
/// Returns the uncompressed stream.
// Mirrors dao/A3SFilesAccessor.java:27-30
pub(crate) fn encode_checked<T: JavaCodec>(
    file: A3sFile,
    value: &T,
) -> Result<Vec<u8>, BuildError> {
    let raw =
        a3s_types::encode_stream(value).map_err(|source| BuildError::Encode { file, source })?;
    if raw.len() as u64 > MAX_STREAM_BYTES {
        return Err(BuildError::LimitExceeded {
            file,
            source: a3s_jserial::Error::LimitExceeded("stream size"),
        });
    }
    a3s_jserial::java_filter_stats(&raw)
        .and_then(|stats| stats.check(JavaFilterLimits::JAVA_A3S))
        .map_err(|source| BuildError::LimitExceeded { file, source })?;
    Ok(raw)
}

/// Decodes a stream produced by [`encode_checked`].
pub(crate) fn decode<T: JavaCodec>(file: A3sFile, raw: &[u8]) -> Result<T, BuildError> {
    a3s_types::decode_stream(raw).map_err(|source| BuildError::Encode { file, source })
}

/// Creates `<root>/.a3s` (one level, like `File.mkdir`).
// Mirrors dao/repository/RepositoryBuildProcessor.java:320-324
pub(crate) fn create_a3s_folder(folder: &Path) -> Result<(), BuildError> {
    if let Err(source) = std::fs::create_dir(folder)
        && !folder.is_dir()
    {
        return Err(BuildError::CreateA3sFolder {
            path: folder.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// Writes one `.a3s` file atomically from the stream [`encode_checked`] validated.
pub(crate) fn write(folder: &Path, file: A3sFile, raw: &[u8]) -> Result<(), BuildError> {
    write_a3s_stream(&folder.join(file.file_name()), raw)
        .map_err(|source| BuildError::Write { file, source })
}
