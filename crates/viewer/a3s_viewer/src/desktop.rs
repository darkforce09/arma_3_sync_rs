//! The desktop the window integrates with: the environment, the special folders, the registry
//! and the log folder. Tests pass an isolated one so no setting touches the real home folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_platform::autostart::RegistryRw;
use a3s_platform::folders::{MemoryFolders, SpecialFolders, SystemFolders};
use a3s_platform::registry::{MemoryRegistry, SystemRegistry};

/// Where the Settings window reads and writes outside the client data: autostart entries,
/// desktop shortcuts, the Arma 3 user folder and the logs.
#[derive(Clone)]
pub struct Desktop {
    /// The environment; `None` when it could not be read (autostart and shortcuts are then
    /// unavailable).
    pub env: Option<PlatformEnv>,

    /// Documents, Desktop and home.
    pub folders: Arc<dyn SpecialFolders>,

    /// The Windows registry (autostart, game detection).
    pub registry: Arc<dyn RegistryRw>,

    /// The folder of the log files, for "Show in folder".
    pub log_dir: Option<PathBuf>,
}

impl Desktop {
    /// The real desktop of the current user.
    pub fn system(log_dir: Option<PathBuf>) -> Self {
        let env = PlatformEnv::current()
            .map_err(|err| tracing::warn!("Cannot read the environment: {err}"))
            .ok();
        Self {
            env,
            folders: Arc::new(SystemFolders),
            registry: Arc::new(SystemRegistry),
            log_dir,
        }
    }

    /// A desktop inside `home`: Documents and Desktop are its subfolders, the registry is in
    /// memory and nothing outside `home` is read or written.
    pub fn isolated(home: &Path) -> Self {
        Self {
            env: Some(PlatformEnv::for_tests(
                Os::current(),
                Some(home.to_path_buf()),
                &[],
                None,
            )),
            folders: Arc::new(MemoryFolders {
                home: Some(home.to_path_buf()),
                documents: Some(home.join("Documents")),
                desktop: Some(home.join("Desktop")),
            }),
            registry: Arc::new(MemoryRegistry::default()),
            log_dir: Some(home.join("logs")),
        }
    }
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desktop")
            .field("env", &self.env)
            .field("log_dir", &self.log_dir)
            .finish_non_exhaustive()
    }
}
