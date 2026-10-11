//! Starting the updated application after a GUI update.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::UpdateContext;
use super::install::release_binary_name;

/// The program, arguments and working directory to start after an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
}

impl RelaunchPlan {
    /// The installed binary (`<install_dir>/arma3sync[.exe]`, else the path the application was
    /// started from), with `-dev` in dev mode.
    // Mirrors a3sUpdater/ui/Updater.java:176-198
    pub fn for_context(ctx: &UpdateContext) -> Self {
        let installed = ctx.install_dir.join(release_binary_name(ctx.os));
        let program = if installed.is_file() {
            installed
        } else {
            ctx.exe_path.clone()
        };
        Self {
            program,
            args: if ctx.dev {
                vec!["-dev".to_owned()]
            } else {
                Vec::new()
            },
            working_dir: ctx.install_dir.clone(),
        }
    }
}

/// Starts `plan` detached, without waiting for it; the caller exits right after, also when this
/// fails.
///
/// `release_instance_lock` runs first; it must release the application's single-instance lock
/// (shut down its instance guard), otherwise the new process finds the old one still running.
pub fn relaunch(plan: &RelaunchPlan, release_instance_lock: impl FnOnce()) -> io::Result<()> {
    release_instance_lock();
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = command.spawn()?;
    // Reaps the child if it exits while this process still runs, so it never lingers as a
    // zombie.
    std::thread::Builder::new()
        .name("a3s-relaunch-reaper".to_owned())
        .spawn(move || {
            let _ignored: io::Result<std::process::ExitStatus> = child.wait();
        })?;
    Ok(())
}
