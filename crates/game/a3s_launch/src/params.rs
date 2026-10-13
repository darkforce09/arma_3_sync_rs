//! The Arma 3 command-line parameters of a profile, in Java's exact order.
//!
//! Arma 3 sees every parameter, so the order, spelling and `-mod=` grouping follow the Java
//! client exactly, quirks included.

use a3s_addons::{AddonRegistry, arma3_directory};
use a3s_types::java_string::java_trim;
use a3s_types::local::{Addon, Configuration, Profile, TreeDirectory, TreeNode};

/// How addon folder paths are written into `-mod=` parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModPathStyle {
    /// The path as is, joined to the addon name with `separator` (Java: `File.separator`).
    Native { separator: char },

    /// A Unix path mapped into Wine's `Z:` drive, for a game running under Proton.
    WineZDrive,
}

impl ModPathStyle {
    /// The host's own style.
    pub fn native() -> Self {
        Self::Native {
            separator: std::path::MAIN_SEPARATOR,
        }
    }

    /// `path` as the game sees it.
    pub fn map_path(self, path: &str) -> String {
        match self {
            Self::Native { .. } => path.to_owned(),
            Self::WineZDrive => wine_path(path),
        }
    }

    /// The separator between a mapped path and an addon name.
    pub fn separator(self) -> char {
        match self {
            Self::Native { separator } => separator,
            Self::WineZDrive => '\\',
        }
    }
}

/// An absolute Unix path in Wine's `Z:` drive (`/home/u` → `Z:\home\u`); any other path is
/// returned unchanged.
pub fn wine_path(unix_path: &str) -> String {
    if unix_path.starts_with('/') {
        format!("Z:{}", unix_path.replace('/', "\\"))
    } else {
        unix_path.to_owned()
    }
}

/// What the run parameters are computed from.
#[derive(Clone, Copy, Debug)]
pub struct RunParameterInput<'a> {
    /// The active profile; no profile gives no parameters.
    pub profile: Option<&'a Profile>,

    /// The server to join (`server_name`) and the favourite servers.
    pub configuration: &'a Configuration,

    /// The addons found on disk.
    pub registry: &'a AddonRegistry,

    /// How mod paths are written.
    pub mod_path_style: ModPathStyle,
}

/// `profile.additional_parameters` split like `StringTokenizer(s, " ")`: on spaces only (runs of
/// spaces are one delimiter; tabs and newlines stay in tokens), each token Java-trimmed.
// Mirrors service/LaunchService.java:72-78
pub fn tokenize_parameters(s: &str) -> Vec<String> {
    s.split(' ')
        .filter(|token| !token.is_empty())
        .map(|token| java_trim(token).to_owned())
        .collect()
}

/// The profile's additional parameters ([`tokenize_parameters`]), every token containing
/// `malloc` (ignoring case) moved to the front, so the last such token comes first.
// Mirrors service/LaunchService.java:427-451
pub fn additional_parameters(profile: Option<&Profile>) -> Vec<String> {
    let Some(text) = profile.and_then(|profile| profile.additional_parameters.as_deref()) else {
        return Vec::new();
    };
    let mut params = Vec::new();
    for token in tokenize_parameters(text) {
        if token.to_lowercase().contains("malloc") {
            params.insert(0, token);
        } else {
            params.push(token);
        }
    }
    params
}

/// Every parameter the game is started with: the additional parameters, then
/// [`run_parameters`].
// Mirrors service/LaunchService.java:211-223
pub fn determine_run_parameters(input: &RunParameterInput<'_>) -> Vec<String> {
    let mut params = additional_parameters(input.profile);
    params.extend(run_parameters(input));
    params
}

/// The launcher options, the server join and the mods of the profile, in Java's order.
// Mirrors service/LaunchService.java:225-293
pub fn run_parameters(input: &RunParameterInput<'_>) -> Vec<String> {
    let Some(profile) = input.profile else {
        return Vec::new();
    };
    let options = &profile.launcher_options;
    let mut params = Vec::new();

    let mut push_value = |prefix: &str, value: Option<&String>| {
        if let Some(value) = value {
            params.push(format!("{prefix}{value}"));
        }
    };
    push_value("-malloc=", options.malloc_selection.as_ref());
    push_value("-name=", options.game_profile.as_ref());

    let flags = [
        (options.show_script_errors, "-showScriptErrors"),
        (options.no_pause, "-noPause"),
        (options.file_patching, "-filePatching"),
        (options.window_mode, "-window"),
        (options.check_signatures, "-checkSignatures"),
    ];
    params.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, flag)| (*flag).to_owned()),
    );

    if options.mission_file
        && let Some(path) = options.mission_file_path.as_deref()
        && !path.is_empty()
    {
        params.push(input.mod_path_style.map_path(path));
    }
    if let Some(value) = &options.max_memory_selection {
        params.push(format!("-maxMem={value}"));
    }
    if options.cpu_count_selection != 0 {
        params.push(format!("-cpuCount={}", options.cpu_count_selection));
    }
    if let Some(value) = &options.ex_threads_selection {
        params.push(format!("-exThreads={value}"));
    }
    let performance = [
        (options.enable_ht, "-enableHT"),
        (options.huge_pages, "-hugepages"),
        (options.no_splash_screen, "-nosplash"),
        (options.default_world, "-world=empty"),
        (options.nologs, "-nologs"),
    ];
    params.extend(
        performance
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, flag)| (*flag).to_owned()),
    );

    params.extend(server_parameters(input.configuration));

    let names = launch_addon_names(profile);
    let addons: Vec<Addon> = names
        .iter()
        .filter_map(|name| input.registry.get(name))
        .map(|addon| Addon::new(addon.key.clone(), addon.name.clone(), addon.path.clone()))
        .collect();
    params.extend(mod_parameters(
        &addons,
        options.arma3_exe_path.as_deref(),
        input.mod_path_style,
    ));
    params
}

/// `-connect=`, `-port=` and `-password=` of the first favourite server named exactly like
/// `configuration.server_name`. A server without a name never matches, one without an address
/// is skipped with a warning, and an empty or missing password gives no `-password=`.
// Mirrors service/LaunchService.java:295-311
fn server_parameters(configuration: &Configuration) -> Vec<String> {
    let Some(server_name) = configuration.server_name.as_deref() else {
        return Vec::new();
    };
    let Some(server) = configuration
        .favorite_servers
        .iter()
        .find(|server| server.name.as_deref() == Some(server_name))
    else {
        return Vec::new();
    };
    let Some(ip) = server.ip_address.as_deref() else {
        tracing::warn!("favourite server {server_name:?} has no address; not joining it");
        return Vec::new();
    };
    let mut params = vec![format!("-connect={ip}"), format!("-port={}", server.port)];
    if let Some(password) = server.password.as_deref().filter(|p| !p.is_empty()) {
        params.push(format!("-password={password}"));
    }
    params
}

/// The names of every selected leaf of `tree`, depth first in child order. Group selection flags
/// are ignored.
// Mirrors service/LaunchService.java:453-466
pub fn selected_addon_names(tree: &TreeDirectory) -> Vec<String> {
    let mut names = Vec::new();
    collect_selected(tree, &mut names);
    names
}

fn collect_selected(directory: &TreeDirectory, names: &mut Vec<String>) {
    for node in &directory.children {
        match node {
            TreeNode::Leaf(leaf) => {
                if leaf.selected {
                    names.push(leaf.name.clone());
                }
            }
            TreeNode::Directory(child) => collect_selected(child, names),
        }
    }
}

/// The lowercase addon keys to launch: the selected ones in priority order, then the remaining
/// selected ones sorted like a Java `TreeSet<String>` (UTF-16 code units).
// Mirrors service/LaunchService.java:313-341
pub fn launch_addon_names(profile: &Profile) -> Vec<String> {
    let mut selected: Vec<String> = selected_addon_names(&profile.tree)
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    selected.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    selected.dedup();

    let mut run: Vec<String> = profile
        .addon_names_by_priority
        .iter()
        .map(|name| name.to_lowercase())
        .filter(|name| selected.contains(name))
        .collect();
    for name in selected {
        if !run.contains(&name) {
            run.push(name);
        }
    }
    run
}

/// The `-mod=` parameters of the resolved addons, in order.
///
/// Consecutive addons with the same path form a group. A group whose first addon lies in the
/// Arma 3 folder (the parent of `arma3_exe_path`, compared ignoring case) gives one
/// `-mod=a;b;` parameter of bare names; every other addon gives its own
/// `-mod=<path><separator><name>;` (`-mod=<name>;` for an empty path). The joined text is then
/// split on the literal `-mod=`, as Java does, so a name or path containing `-mod=` splits too.
// Mirrors service/LaunchService.java:357-421
pub fn mod_parameters(
    addons: &[Addon],
    arma3_exe_path: Option<&str>,
    style: ModPathStyle,
) -> Vec<String> {
    let root = arma3_directory(arma3_exe_path)
        .and_then(|parent| std::path::absolute(&parent).ok())
        .map(|parent| parent.to_string_lossy().to_lowercase());
    let at_root = |addon: &Addon| {
        root.as_deref()
            .is_some_and(|root| addon.path.to_lowercase() == root)
    };

    let mut mods = String::new();
    for group in addons.chunk_by(|a, b| a.path == b.path) {
        if at_root(&group[0]) {
            mods.push_str("-mod=");
            for addon in group {
                mods.push_str(&addon.name);
                mods.push(';');
            }
            continue;
        }
        for addon in group {
            mods.push_str("-mod=");
            if !addon.path.is_empty() {
                mods.push_str(&style.map_path(&addon.path));
                mods.push(style.separator());
            }
            mods.push_str(&addon.name);
            mods.push(';');
        }
    }

    mods.split("-mod=")
        .filter(|piece| !java_trim(piece).is_empty())
        .map(|piece| format!("-mod={piece}"))
        .collect()
}

/// The Launcher Options preview: each parameter followed by a line break.
// Mirrors ui/main/LauncherOptionsPanel.java:851-863
pub fn run_parameters_display(params: &[String]) -> String {
    params.iter().map(|param| format!("{param}\n")).collect()
}

/// The arguments of a desktop shortcut: each parameter preceded by a space.
// Mirrors ui/main/MainPanel.java:715-719
pub fn shortcut_arguments(params: &[String]) -> String {
    params.iter().map(|param| format!(" {param}")).collect()
}
