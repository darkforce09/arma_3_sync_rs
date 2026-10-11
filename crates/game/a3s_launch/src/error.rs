/// Why a launch failed. `Display` is the text the launch dialog shows.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// No profile, or its Arma 3 executable is unset or does not exist.
    // Mirrors service/LaunchService.java:43-56
    #[error(
        "ArmA 3 Executable location is wrong or missing.\nPlease checkout Launcher Options panel."
    )]
    ExecutableLocation,

    /// The executable cannot be run on this OS (by its file name).
    // Mirrors dao/LauncherDAO.java:179-181, 287-289, 346-348
    #[error("{name} - invalid executable file.")]
    InvalidExecutable {
        /// File name of the executable.
        name: String,
    },

    /// The process could not be started.
    #[error("Cannot run program \"{program}\": {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    /// Waiting for the process failed.
    #[error("Failed to wait for \"{program}\": {source}")]
    Wait {
        program: String,
        #[source]
        source: std::io::Error,
    },

    /// The launch thread could not be created.
    #[error("Cannot start the launch thread: {0}")]
    Thread(#[source] std::io::Error),

    /// A Steam launch was requested but no Steam client was found.
    #[error("Steam was not found. Install Steam or select the direct launch mode.")]
    SteamNotFound,

    /// After a Steam launch, the game process never appeared, so auto-restart cannot follow it.
    #[error("ArmA 3 did not start within {secs} s; auto-restart stopped.")]
    SteamGameNotSeen { secs: u64 },

    /// Auto-restart stopped because the game kept exiting right after starting.
    #[error("Auto-restart stopped: ArmA 3 exited within {secs} s {count} times in a row.")]
    RestartLoop { secs: u64, count: u32 },
}
