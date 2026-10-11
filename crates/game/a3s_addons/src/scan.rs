use std::collections::HashSet;
use std::path::{Path, PathBuf};

use a3s_types::constants::{GAME_SYSTEM_FOLDERS, GameDlc};
use a3s_types::java_string::compare_ignore_case;
use a3s_types::local::{Profile, TreeDirectory, TreeLeaf, TreeNode};

use crate::AVAILABLE_ROOT_NAME;
use crate::registry::AddonRegistry;
use crate::tree::{normalize_group_tree, sort_children};

/// File-name fragments that make an `addons` folder an Arma 3 mod.
// Mirrors dao/DataAccessConstants.java:24-25
const ADDON_FILE_MARKERS: [&str; 2] = [".pbo", ".ebo"];

/// How paths are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    /// Compare paths ignoring case, as the Java client does on Windows. Defaults to
    /// `cfg!(windows)`.
    pub case_insensitive_paths: bool,
}

#[expect(
    clippy::derivable_impls,
    reason = "the default depends on the target platform"
)]
impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            case_insensitive_paths: cfg!(windows),
        }
    }
}

impl ScanOptions {
    /// `path` made absolute (not canonicalised) as a string, lowercased when comparing
    /// ignoring case.
    pub(crate) fn path_key(self, path: &Path) -> String {
        self.compare_form(&absolute(path).to_string_lossy())
    }

    pub(crate) fn compare_form(self, s: &str) -> String {
        if self.case_insensitive_paths {
            s.to_lowercase()
        } else {
            s.to_owned()
        }
    }
}

/// What [`scan`] found.
#[derive(Clone, Debug)]
pub struct ScanResult {
    /// Every mod found, then every CDLC of the Arma 3 folder.
    pub registry: AddonRegistry,

    /// The available-addons tree (root [`AVAILABLE_ROOT_NAME`]): directories leading to mods,
    /// mods as leaves named by their registry key.
    pub tree: TreeDirectory,

    /// The CDLCs present in the Arma 3 folder, in [`GameDlc::ALL`] order.
    pub dlcs: Vec<GameDlc>,
}

/// `path` made absolute against the working directory, without resolving links or `..`
/// (Java `File.getAbsolutePath`).
pub(crate) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The Arma 3 folder: the parent of the configured executable. `None` when no executable is set
/// or it has no parent.
// Mirrors service/AddonService.java:52-62
pub fn arma3_directory(arma3_exe_path: Option<&str>) -> Option<PathBuf> {
    let exe = arma3_exe_path.filter(|p| !p.is_empty())?;
    let parent = Path::new(exe).parent()?;
    (!parent.as_os_str().is_empty()).then(|| parent.to_path_buf())
}

/// The game's own folders below `arma3_dir`, which are never scanned for mods, in the form
/// [`ScanOptions`] compares paths.
// Mirrors service/AddonService.java:64-75
pub fn excluded_system_paths(arma3_dir: &Path, opts: &ScanOptions) -> Vec<String> {
    GAME_SYSTEM_FOLDERS
        .iter()
        .map(|name| opts.path_key(&arma3_dir.join(name)))
        .collect()
}

/// The parent of an existing `path`, made absolute; `None` when `path` or its parent does not
/// exist or it has no parent (a filesystem root or a single relative component).
fn existing_parent(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if !path.exists() {
        return None;
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    parent.exists().then(|| absolute(parent))
}

/// The search directories that are scanned, in profile order, with overlapping ones collapsed.
///
/// A directory is dropped when it or its parent does not exist, or when it has no parent (so a
/// filesystem root is never scanned). Of two directories whose parents differ and whose path
/// strings contain one another (a substring test, not a path-prefix test), the shorter one is
/// kept. Directories with the same parent are never collapsed. Duplicates appear once.
// Mirrors service/AddonService.java:93-149
pub fn effective_search_directories(dirs: &[String], opts: &ScanOptions) -> Vec<String> {
    let utf16_len = |s: &str| s.encode_utf16().count();
    let mut out: Vec<String> = Vec::new();
    for i in dirs {
        let Some(i_parent) = existing_parent(i) else {
            continue;
        };
        let i_cmp = opts.compare_form(i);
        let mut keep = i;
        for j in dirs {
            let Some(j_parent) = existing_parent(j) else {
                continue;
            };
            if i_parent.as_os_str() == j_parent.as_os_str() {
                continue;
            }
            let j_cmp = opts.compare_form(j);
            if (i_cmp.contains(&j_cmp) || j_cmp.contains(&i_cmp)) && utf16_len(j) < utf16_len(keep)
            {
                keep = j;
            }
        }
        if !out.contains(keep) {
            out.push(keep.clone());
        }
    }
    out
}

/// The last component of `path` (Java `File.getName`), lossily converted.
fn file_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => lossy_name(name, path),
        None => path.to_string_lossy().into_owned(),
    }
}

fn lossy_name(name: &std::ffi::OsStr, path: &Path) -> String {
    if let Some(s) = name.to_str() {
        return s.to_owned();
    }
    tracing::warn!(path = %path.display(), "file name is not valid UTF-8; using a lossy name");
    name.to_string_lossy().into_owned()
}

/// The entries of `dir` sorted by name ignoring case, ties in byte order; `None` when `dir`
/// cannot be listed.
fn sorted_entries(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => {
            tracing::debug!(path = %dir.display(), error = %e, "cannot list directory");
            return None;
        }
    };
    let mut entries: Vec<(String, PathBuf)> = read
        .filter_map(Result::ok)
        .map(|e| {
            let path = e.path();
            (lossy_name(&e.file_name(), &path), path)
        })
        .collect();
    entries.sort_by(|a, b| compare_ignore_case(&a.0, &b.0).then_with(|| a.0.cmp(&b.0)));
    Some(entries)
}

/// Whether `dir` is an Arma 3 mod.
///
/// A mod has a child named `addons` (any case) that is a listable
/// directory with an entry whose name contains `.pbo` or `.ebo` (case-sensitive, anywhere in
/// the name, so `x.pbo.zip` counts and `X.PBO` does not).
// Mirrors service/AddonService.java:307-323
pub fn is_mod_directory(dir: &Path) -> bool {
    sorted_entries(dir).is_some_and(|entries| contains_addons_folder(&entries))
}

fn contains_addons_folder(entries: &[(String, PathBuf)]) -> bool {
    entries
        .iter()
        .filter(|(name, _)| name.to_lowercase() == "addons")
        .any(|(_, path)| {
            sorted_entries(path).is_some_and(|files| {
                files
                    .iter()
                    .any(|(name, _)| ADDON_FILE_MARKERS.iter().any(|m| name.contains(m)))
            })
        })
}

struct Scanner<'a> {
    registry: &'a mut AddonRegistry,
    excluded: &'a [String],
    opts: ScanOptions,

    /// Canonical paths of the directories being scanned, to stop at symbolic-link loops.
    stack: Vec<PathBuf>,
}

impl Scanner<'_> {
    /// Scans `entries`, the content of the directory whose canonical path is `canonical`, into
    /// `out`.
    fn scan_children(
        &mut self,
        canonical: Option<PathBuf>,
        entries: &[(String, PathBuf)],
        out: &mut TreeDirectory,
    ) {
        let pushed = canonical.is_some();
        self.stack.extend(canonical);
        for (name, path) in entries {
            if let Some(node) = self.scan_entry(name, path) {
                out.children.push(node);
            }
        }
        if pushed {
            self.stack.pop();
        }
        sort_children(out);
    }

    /// The node for `entry` in the available-addons tree: a leaf named by its new registry key
    /// for a mod (whose sub-directories are not scanned), a directory for a folder with mods
    /// below it, `None` otherwise.
    // Mirrors service/AddonService.java:292-348 and :359-372
    fn scan_entry(&mut self, name: &str, entry: &Path) -> Option<TreeNode> {
        if !entry.is_dir() || self.excluded.contains(&self.opts.path_key(entry)) {
            return None;
        }
        let canonical = std::fs::canonicalize(entry).ok();
        if let Some(c) = &canonical
            && self.stack.contains(c)
        {
            tracing::warn!(path = %entry.display(), "symbolic link loop; not scanned");
            return None;
        }
        let entries = sorted_entries(entry);
        if entries.as_deref().is_some_and(contains_addons_folder) {
            let parent = entry.parent().map(absolute).unwrap_or_default();
            let key = self
                .registry
                .register(name, parent.to_string_lossy().into_owned());
            return Some(TreeNode::Leaf(TreeLeaf::new(key)));
        }
        let mut dir = TreeDirectory::new(name);
        self.scan_children(canonical, entries.as_deref().unwrap_or_default(), &mut dir);
        (!dir.children.is_empty()).then_some(TreeNode::Directory(dir))
    }
}

/// Scans the profile's addon search directories and the Arma 3 folder.
///
/// Search directories are scanned in profile order (that order decides which of two same-named
/// mods gets the plain key), each directory's entries sorted by name. Mods are registered first,
/// then each CDLC folder present in the Arma 3 folder under its Java name, so a mod folder named
/// `GM` keeps `GM` and the DLC becomes `GM*`. With a single search directory its own level is
/// hidden in the tree; otherwise each search directory with mods appears under its folder name,
/// and only the first of two same-named ones is shown (its mods stay registered).
///
/// Blocking filesystem walk; unreadable directories count as empty.
// Mirrors service/AddonService.java:47-184 and :230-269
pub fn scan(profile: &Profile, opts: &ScanOptions) -> ScanResult {
    let mut registry = AddonRegistry::new();
    let arma3_dir = arma3_directory(profile.launcher_options.arma3_exe_path.as_deref());
    let excluded = arma3_dir
        .as_deref()
        .map(|dir| excluded_system_paths(dir, opts))
        .unwrap_or_default();

    let mut search_roots = Vec::new();
    {
        let mut scanner = Scanner {
            registry: &mut registry,
            excluded: &excluded,
            opts: *opts,
            stack: Vec::new(),
        };
        for dir in effective_search_directories(&profile.addon_search_directories, opts) {
            let path = Path::new(&dir);
            if !path.exists() {
                continue;
            }
            let mut node = TreeDirectory::new(file_name(path));
            let entries = sorted_entries(path).unwrap_or_default();
            scanner.scan_children(std::fs::canonicalize(path).ok(), &entries, &mut node);
            search_roots.push(node);
        }
    }

    let mut tree = TreeDirectory::new(AVAILABLE_ROOT_NAME);
    if search_roots.len() == 1 {
        tree.children = search_roots.remove(0).children;
    } else {
        tree.children = search_roots
            .into_iter()
            .filter(|d| !d.children.is_empty())
            .map(TreeNode::Directory)
            .collect();
    }
    sort_children(&mut tree);
    normalize_group_tree(&mut tree);

    let mut dlcs = Vec::new();
    if let Some(dir) = &arma3_dir {
        let path = absolute(dir).to_string_lossy().into_owned();
        for dlc in GameDlc::ALL {
            if dir.join(dlc.java_name()).exists() {
                registry.register(dlc.java_name(), path.clone());
                dlcs.push(dlc);
            }
        }
    }

    ScanResult {
        registry,
        tree,
        dlcs,
    }
}

/// The flat list view of an available-addons tree: every leaf once (the first of each exact
/// name in depth-first order) under a new root, sorted ignoring case.
// Mirrors service/AddonService.java:221-228 and :271-290
pub fn available_addons_list(tree: &TreeDirectory) -> TreeDirectory {
    fn collect(dir: &TreeDirectory, seen: &mut HashSet<String>, out: &mut TreeDirectory) {
        for child in &dir.children {
            match child {
                TreeNode::Leaf(leaf) => {
                    if seen.insert(leaf.name.clone()) {
                        out.children.push(TreeNode::Leaf(leaf.clone()));
                    }
                }
                TreeNode::Directory(d) => collect(d, seen, out),
            }
        }
    }
    let mut out = TreeDirectory::new(AVAILABLE_ROOT_NAME);
    collect(tree, &mut HashSet::new(), &mut out);
    sort_children(&mut out);
    out
}
