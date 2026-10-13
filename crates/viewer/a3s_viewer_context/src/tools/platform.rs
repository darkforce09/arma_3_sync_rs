use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use a3s_foundation::paths::PlatformEnv;
use a3s_platform::detect;
use a3s_platform::process::{self, ProcessProbe, SystemProcessProbe};
use a3s_platform::registry::{RegistryRead, SystemRegistry};

/// How the tools reach the operating system: environment, registry and process list. Tests
/// replace it with fixed values.
#[derive(Clone)]
pub struct Platform {
    /// The process environment; `None` when it could not be read.
    pub env: Option<PlatformEnv>,

    /// The Windows registry.
    pub registry: Arc<dyn RegistryRead>,

    /// The running processes.
    pub processes: Arc<dyn ProcessProbe>,
}

impl Platform {
    /// The real environment, registry and process list.
    pub fn system() -> Self {
        Self {
            env: PlatformEnv::current()
                .map_err(|err| tracing::warn!("Cannot read the environment: {err}"))
                .ok(),
            registry: Arc::new(SystemRegistry),
            processes: Arc::new(SystemProcessProbe),
        }
    }

    /// The `TeamSpeak` 3 folder recorded by its installer (Windows only).
    pub fn ts3_dir(&self) -> Option<PathBuf> {
        detect::ts3_dir(self.env.as_ref()?, &*self.registry)
    }

    /// The default Arma 3 RPT log folder. Reads Steam's library files on Linux.
    pub fn rpt_dir(&self) -> Option<PathBuf> {
        detect::rpt_dir(self.env.as_ref()?, &*self.registry)
    }

    /// Whether a `TeamSpeak` 3 client runs.
    pub fn is_ts3_running(&self) -> bool {
        process::is_ts3_running(&*self.processes)
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::system()
    }
}

impl fmt::Debug for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Platform")
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}
