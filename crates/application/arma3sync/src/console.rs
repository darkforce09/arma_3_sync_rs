//! The interactive console: commands and their answers are read line by line from standard
//! input, so a script can pipe them in.
//!
//! The console ends on `QUIT` or at the end of the input. It fails (exit code 1) when a command
//! failed, a command was unknown, or the input ended in the middle of a command.

use std::io::{BufRead, Write as _};
use std::path::Path;

use a3s_repo_files::services::ProtocolInput;
use a3s_types::ProtocolType;
use a3s_types::constants::ConsoleCommand;

use crate::app::{App, CommandResult, Failed};
use crate::commands::{self, report};
use crate::{build_info, commands::update};

/// The input ended before a command had all its answers.
struct EndOfInput;

/// Result of one console command.
type Step = Result<CommandResult, EndOfInput>;

/// Characters a repository name must not contain.
// Mirrors console/CommandConsole.java:183-191
const FORBIDDEN_NAME_CHARACTERS: [char; 8] = ['/', '\\', '*', '?', '"', '<', '>', '|'];

/// Protocols a repository can be created with.
const PROTOCOLS: [ProtocolType; 7] = [
    ProtocolType::Ftp,
    ProtocolType::Ftps,
    ProtocolType::Sftp,
    ProtocolType::Http,
    ProtocolType::Https,
    ProtocolType::HttpWebdav,
    ProtocolType::HttpsWebdav,
];

/// Runs the console until `QUIT` or the end of standard input.
// Mirrors console/CommandConsole.java:33-86
pub(crate) fn run(app: &App, dev: bool) -> CommandResult {
    let stdin = std::io::stdin();
    let mut console = Console {
        app,
        dev,
        input: stdin.lock(),
    };
    display_commands();
    let mut failed = false;
    loop {
        say!("");
        let Ok(line) = console.ask("Please enter a command = ") else {
            break;
        };
        let command = ConsoleCommand::from_java_name(&line.trim().to_ascii_uppercase());
        let step = match command {
            Some(ConsoleCommand::Quit) => {
                say!("");
                say!("ArmA3Sync exited.");
                break;
            }
            Some(command) => console.execute(command),
            None => {
                say_error!("ArmA3Sync - bad command.");
                Ok(Err(Failed))
            }
        };
        match step {
            Ok(Ok(())) => {}
            Ok(Err(Failed)) => failed = true,
            Err(EndOfInput) => {
                say_error!("The input ended before the command was complete.");
                failed = true;
                break;
            }
        }
    }
    if failed { Err(Failed) } else { Ok(()) }
}

struct Console<'a, R> {
    app: &'a App,
    dev: bool,
    input: R,
}

impl<R: BufRead> Console<'_, R> {
    fn execute(&mut self, command: ConsoleCommand) -> Step {
        match command {
            ConsoleCommand::New => self.create(),
            ConsoleCommand::Build => self.build(),
            ConsoleCommand::Check => self.check(),
            ConsoleCommand::Delete => self.delete(),
            ConsoleCommand::List => {
                self.list();
                Ok(Ok(()))
            }
            ConsoleCommand::Sync => self.sync(),
            ConsoleCommand::Extract => self.extract(),
            ConsoleCommand::Update => Ok(self.update()),
            ConsoleCommand::Commands | ConsoleCommand::Help => {
                display_commands();
                Ok(Ok(()))
            }
            ConsoleCommand::Version => {
                say!("");
                say!("{}", build_info::version_banner());
                Ok(Ok(()))
            }
            ConsoleCommand::Quit => Ok(Ok(())),
        }
    }

    /// Prints `prompt` and reads one line, without its line end.
    fn ask(&mut self, prompt: &str) -> Result<String, EndOfInput> {
        let mut stdout = std::io::stdout();
        write!(stdout, "{prompt}").ok();
        stdout.flush().ok();
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) | Err(_) => Err(EndOfInput),
            Ok(_) => {
                let end = line.trim_end_matches(['\r', '\n']).len();
                line.truncate(end);
                Ok(line)
            }
        }
    }

    /// Asks until `accept` takes the answer.
    fn ask_until<T>(
        &mut self,
        prompt: &str,
        mut accept: impl FnMut(&str) -> Option<T>,
    ) -> Result<T, EndOfInput> {
        loop {
            if let Some(value) = accept(&self.ask(prompt)?) {
                return Ok(value);
            }
        }
    }

    /// Asks for a yes/no answer, ignoring case.
    fn ask_yes_no(&mut self, prompt: &str) -> Result<bool, EndOfInput> {
        self.ask_until(prompt, |answer| {
            if answer.eq_ignore_ascii_case("yes") {
                Some(true)
            } else if answer.eq_ignore_ascii_case("no") {
                Some(false)
            } else {
                None
            }
        })
    }

    /// Asks for a non-empty repository name.
    fn ask_repository_name(&mut self) -> Result<String, EndOfInput> {
        self.ask_until("Enter repository name: ", |name| {
            (!name.is_empty()).then(|| name.to_owned())
        })
    }

    /// Asks for the path of an existing folder; an empty answer is `None` when `optional`.
    fn ask_folder(&mut self, prompt: &str, optional: bool) -> Result<Option<String>, EndOfInput> {
        self.ask_until(prompt, |path| {
            if path.is_empty() {
                optional.then_some(None)
            } else if Path::new(path).is_dir() {
                Some(Some(path.to_owned()))
            } else {
                say_error!("Target folder does not exists!");
                None
            }
        })
    }

    /// Asks for paths until an empty answer; each must exist.
    fn ask_existing_paths(&mut self, prompt: &str) -> Result<Vec<String>, EndOfInput> {
        let mut paths = Vec::new();
        loop {
            let path = self.ask(prompt)?;
            if path.is_empty() {
                return Ok(paths);
            }
            if Path::new(&path).exists() {
                paths.push(path);
            } else {
                say_error!("Wrong path, file does not exists.");
            }
        }
    }

    /// Prints every repository with its connection settings; the password is not shown.
    // Mirrors console/CommandConsole.java:88-168
    fn list(&self) {
        say!("");
        say!("List repositories");
        let data = self.app.data.lock();
        let names = data.repository_names();
        say!("Number of repositories found: {}", names.len());
        say!("");
        for repository in data.repositories() {
            let protocol = &repository.protocol;
            let shown = |value: Option<&str>| match value {
                Some(value) if !value.is_empty() => value.to_owned(),
                _ => "null".to_owned(),
            };
            say!("Repository name: {}", repository.name);
            say!("Protocole: {}", protocol.protocol_type.description());
            say!("Url: {}", shown(Some(&protocol.url)));
            say!("Port: {}", protocol.port);
            say!("Login: {}", shown(protocol.login.as_deref()));
            let password_set = protocol.password.as_deref().is_some_and(|p| !p.is_empty());
            say!("Password: {}", if password_set { "(set)" } else { "null" });
            say!(
                "Auto-config url: {}",
                match repository.auto_config_url.as_deref() {
                    Some(url) if !url.is_empty() => {
                        format!("{}{url}", protocol.protocol_type.prompt())
                    }
                    _ => "null".to_owned(),
                }
            );
            say!(
                "Repository main folder path: {}",
                shown(repository.path.as_deref())
            );
            say!("");
        }
    }

    /// Creates a repository from the answers, with no timeouts.
    // Mirrors console/CommandConsole.java:170-354
    fn create(&mut self) -> Step {
        say!("");
        say!("Create a new repository");
        let name = self.ask_until("Enter repository name: ", |name| {
            if name.contains(FORBIDDEN_NAME_CHARACTERS) {
                let listed: String = FORBIDDEN_NAME_CHARACTERS
                    .iter()
                    .map(|c| format!(" {c}"))
                    .collect();
                say_error!("Repository name must not contains special characters like:{listed}");
                None
            } else {
                (!name.is_empty()).then(|| name.to_owned())
            }
        })?;
        let names: Vec<&str> = PROTOCOLS.iter().map(|p| p.description()).collect();
        let protocol_type = self.ask_until(
            &format!("Enter repository protocol ({}): ", names.join(", ")),
            |answer| {
                PROTOCOLS
                    .into_iter()
                    .find(|p| p.description().eq_ignore_ascii_case(answer))
            },
        )?;
        let validate_ssl_certificate = if matches!(
            protocol_type,
            ProtocolType::Ftps | ProtocolType::Https | ProtocolType::HttpsWebdav
        ) {
            self.ask_yes_no("Validate SSL certificate yes/no? ")?
        } else {
            false
        };
        let default_port = protocol_type.default_port();
        let port = self.ask_until(
            &format!("Enter repository port (leave blank for {default_port}): "),
            |port| {
                if port.is_empty() {
                    Some(default_port.to_owned())
                } else {
                    port.parse::<u16>().is_ok().then(|| port.to_owned())
                }
            },
        )?;
        let login = self.ask_until(
            "Enter user login (enter 'anonymous' for public access): ",
            |login| (!login.is_empty()).then(|| login.to_owned()),
        )?;
        let password = self.ask("Enter user password (leave blank if no password): ")?;
        let url = self.ask_until("Enter repository url: ", |url| {
            let url = strip_scheme(url);
            (!url.is_empty()).then(|| url.to_owned())
        })?;
        let path = self.ask_folder("Enter main folder location (leave blank to pass): ", true)?;

        let input = ProtocolInput {
            protocol_type,
            url,
            port,
            login,
            password,
            validate_ssl_certificate,
        };
        let mut data = self.app.data.lock();
        let created = data.create_repository(&name, &input).and_then(|()| {
            data.update_repository(&name, |repository| {
                repository.protocol.connection_timeout = Some("0".to_owned());
                repository.protocol.read_timeout = Some("0".to_owned());
                repository.path = path;
                Ok(true)
            })
        });
        Ok(match created {
            Ok(_) => {
                say!(
                    "Repository creation finished.\nYou can now run the BUILD command to construct the repository"
                );
                Ok(())
            }
            Err(err) => Err(report(&err)),
        })
    }

    /// Asks for the build settings of a repository, stores them and builds it.
    // Mirrors console/CommandConsole.java:356-542
    fn build(&mut self) -> Step {
        say!("");
        say!("Build repository");
        let name = self.ask_repository_name()?;
        let Ok(repository) = commands::repository(self.app, &name) else {
            return Ok(Err(Failed));
        };
        let path = match repository.path.as_deref() {
            Some(path) if !path.is_empty() => None,
            _ => self.ask_folder("Enter repository main folder location: ", false)?,
        };
        let connections = self.ask_until(
            "Set maximum number of client connections (1-10): ",
            |answer| answer.parse::<i32>().ok().filter(|n| (1..=10).contains(n)),
        )?;
        let compressed = self.ask_yes_no("Add compressed pbo files (yes/no): ")?;
        let partial_file_transfer = if repository.protocol.protocol_type == ProtocolType::Ftp {
            true
        } else {
            self.ask_yes_no("Use HTTP partial file transfer (yes/no): ")?
        };
        let excluded_files =
            self.ask_existing_paths("Add file path to exclude from build (leave blank to pass): ")?;
        let excluded_folders = self.ask_existing_paths(
            "Add folder path to exclude extra local content when sync (leave blank to pass): ",
        )?;

        let stored = self.app.data.lock().update_repository(&name, |stored| {
            if let Some(path) = path {
                stored.path = Some(path);
            }
            stored.number_of_connections = connections;
            stored.compressed = compressed;
            stored.set_use_partial_file_transfer(partial_file_transfer);
            stored.excluded_files_from_build = excluded_files.into_iter().collect();
            stored.excluded_folders_from_sync = excluded_folders.into_iter().collect();
            Ok(true)
        });
        if let Err(err) = stored {
            return Ok(Err(report(&err)));
        }
        Ok(commands::build::run(self.app, &name))
    }

    // Mirrors console/CommandConsole.java:544-580
    fn check(&mut self) -> Step {
        say!("");
        say!("Check repository");
        let name = self.ask_repository_name()?;
        Ok(commands::check::run(self.app, &name))
    }

    // Mirrors console/CommandConsole.java:582-623
    fn delete(&mut self) -> Step {
        say!("");
        say!("Delete repository");
        let name = self.ask_repository_name()?;
        let name = name.trim();
        let removed = self.app.data.lock().remove_repository(name);
        Ok(match removed {
            Ok(_) => {
                say!("Repository {name} deleted.");
                Ok(())
            }
            Err(err) => Err(report(&err)),
        })
    }

    // Mirrors console/CommandConsole.java:625-703
    fn sync(&mut self) -> Step {
        say!("");
        say!("Synchronize with repository");
        let name = self.ask_repository_name()?;
        if commands::repository(self.app, &name).is_err() {
            return Ok(Err(Failed));
        }
        let destination = self.ask_until("Enter destination folder path: ", |path| {
            if path.is_empty() {
                None
            } else if Path::new(path).is_dir() {
                Some(path.to_owned())
            } else {
                say_error!("Destination folder does not exists!");
                None
            }
        })?;
        let exact_match = self.ask_yes_no(
            "Perform Exact file matching (yes/no, choosing yes will erase all extra files into the target folder): ",
        )?;
        Ok(commands::sync::run(
            self.app,
            &name,
            &destination,
            exact_match,
        ))
    }

    // Mirrors console/CommandConsole.java:705-729
    fn extract(&mut self) -> Step {
        say!("");
        say!("Extract *.bikey files");
        let source = self.ask_until(
            "Enter source directory to search for *.bikey files: ",
            |path| (!path.is_empty()).then(|| path.to_owned()),
        )?;
        let target = self.ask_until("Enter target directory to copy *.bikey files: ", |path| {
            (!path.is_empty()).then(|| path.to_owned())
        })?;
        Ok(commands::extract::run(&source, &target))
    }

    /// Runs the update; "already up to date" is a success here.
    // Mirrors console/CommandConsole.java:731-739
    fn update(&self) -> CommandResult {
        say!("");
        say!("Check for updates.");
        match update::run(self.app, self.dev) {
            code if code == crate::EXIT_FAILURE => Err(Failed),
            _ => Ok(()),
        }
    }
}

// Mirrors console/CommandConsole.java:33-49
fn display_commands() {
    say!("");
    say!("ArmA3Sync console commands:");
    for (command, description) in [
        (ConsoleCommand::New, "create a new repository"),
        (ConsoleCommand::Build, "build repository"),
        (ConsoleCommand::Check, "check repository synchronization"),
        (ConsoleCommand::Delete, "delete repository"),
        (ConsoleCommand::List, "list repositories"),
        (
            ConsoleCommand::Sync,
            "synchronize content with a repository",
        ),
        (
            ConsoleCommand::Extract,
            "extract *.bikey files from source directory to target directory",
        ),
        (ConsoleCommand::Update, "check for updates"),
        (ConsoleCommand::Commands, "display commands"),
        (ConsoleCommand::Version, "display version"),
        (ConsoleCommand::Quit, "quit"),
    ] {
        say!("{}: {description}", command.java_name());
    }
}

/// `url` without a leading scheme of [`PROTOCOLS`] (any case), as typed otherwise.
// Mirrors console/CommandConsole.java:296-304
fn strip_scheme(url: &str) -> &str {
    PROTOCOLS
        .iter()
        .map(|p| p.prompt())
        .find_map(|prompt| {
            url.get(..prompt.len())
                .filter(|head| head.eq_ignore_ascii_case(prompt))
                .map(|_| &url[prompt.len()..])
        })
        .unwrap_or(url)
}
