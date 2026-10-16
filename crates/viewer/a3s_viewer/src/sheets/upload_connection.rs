//! The Upload connection sheet of a repository built on this computer: where a publish uploads
//! to.

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::ProtocolInput;
use a3s_types::{Protocol, ProtocolType};
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::{form_row, segmented};
use std::sync::mpsc;

use a3s_sync::EventError;
use a3s_transport::{CancellationToken, TransferCtx};
use a3s_types::local::protocol::{check_protocol, new_protocol};
use a3s_viewer_context::{Change, ViewerContext, admin};
use egui::{Label, RichText};

use super::Shown;
use super::add_repository::{spawn, strip_scheme, take, text_row};
use crate::toasts::Toaster;

/// The protocols a repository can be uploaded with, in the order of the segmented control.
const PROTOCOLS: [(&str, ProtocolType); 5] = [
    ("FTP", ProtocolType::Ftp),
    ("FTPS", ProtocolType::Ftps),
    ("SFTP", ProtocolType::Sftp),
    ("WebDAV", ProtocolType::HttpWebdav),
    ("WebDAV HTTPS", ProtocolType::HttpsWebdav),
];

/// The login of an anonymous connection.
const ANONYMOUS: &str = "anonymous";

/// The sheet's state.
pub(crate) struct UploadConnection {
    repository: String,
    protocol: usize,
    host: String,
    path: String,
    port: String,
    login: String,
    password: String,
    timeouts: (Option<String>, Option<String>),
    error: Option<String>,
    test: Option<mpsc::Receiver<Result<bool, EventError>>>,
    test_result: Option<Result<bool, String>>,

    /// Cancels the test when the sheet closes.
    cancel: CancellationToken,
}

impl Drop for UploadConnection {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl UploadConnection {
    /// The sheet for `repository`, filled with its upload connection, or else with its
    /// download address and FTP.
    pub(crate) fn new(viewer: &ViewerContext, repository: &str) -> Self {
        let state = viewer.read();
        let stored = state.client.repository(repository).ok();
        let upload = stored.and_then(|stored| stored.upload_protocol.clone());
        let mut sheet = Self {
            repository: repository.to_owned(),
            protocol: 0,
            host: String::new(),
            path: String::new(),
            port: ProtocolType::Ftp.default_port().to_owned(),
            login: String::new(),
            password: String::new(),
            timeouts: (None, None),
            error: None,
            test: None,
            test_result: None,
            cancel: CancellationToken::new(),
        };
        match (upload, stored) {
            (Some(protocol), _) => sheet.fill(&protocol),
            (None, Some(stored)) => {
                let (host, path) = split_url(&stored.protocol.url);
                sheet.host = host;
                sheet.path = path;
            }
            (None, None) => {}
        }
        sheet
    }

    fn fill(&mut self, protocol: &Protocol) {
        self.protocol = PROTOCOLS
            .iter()
            .position(|(_, t)| *t == protocol.protocol_type)
            .unwrap_or(0);
        let (host, path) = split_url(&protocol.url);
        self.host = host;
        self.path = path;
        self.port.clone_from(&protocol.port);
        let login = protocol.login.clone().unwrap_or_default();
        self.login = if login == ANONYMOUS {
            String::new()
        } else {
            login
        };
        self.password = protocol.password.clone().unwrap_or_default();
        self.timeouts = (
            protocol.connection_timeout.clone(),
            protocol.read_timeout.clone(),
        );
    }

    fn input(&self) -> ProtocolInput {
        let host = strip_scheme(self.host.trim()).trim_matches('/');
        let path = self.path.trim().trim_matches('/');
        let url = if path.is_empty() {
            host.to_owned()
        } else {
            format!("{host}/{path}")
        };
        let login = self.login.trim();
        ProtocolInput {
            protocol_type: PROTOCOLS[self.protocol].1,
            url,
            port: self.port.trim().to_owned(),
            login: if login.is_empty() {
                ANONYMOUS.to_owned()
            } else {
                login.to_owned()
            },
            password: self.password.clone(),
            validate_ssl_certificate: true,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        if let Some(result) = take(&mut self.test) {
            self.test_result = Some(result.map_err(|err| err.message));
        }
        let action = Dialog::new("a3s_viewer::sheet::upload_connection", "Upload connection")
            .ok_label("Save")
            .ok_enabled(!self.host.trim().is_empty())
            .default_button(Some(DialogButton::Ok))
            .show(ctx, |ui| self.body(ui, viewer));
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => {
                let input = self.input();
                let (connection_timeout, read_timeout) = self.timeouts.clone();
                let saved = viewer.mutate(Some(Change::Repository), |state| {
                    state.client.set_upload_protocol(
                        &self.repository,
                        &input,
                        connection_timeout,
                        read_timeout,
                    )
                });
                match saved {
                    Ok(()) => {
                        toasts.success("Saved the upload connection");
                        Shown::Closed(None)
                    }
                    Err(err) => {
                        self.error = Some(err.user_message());
                        Shown::Open
                    }
                }
            }
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, viewer: &ViewerContext) {
        let palette = theme::palette(ui.ctx());
        ui.set_width(440.0);
        ui.label("Where Publish uploads the repository.");
        ui.add_space(6.0);
        frames::group(ui, |ui| {
            form_row(ui, "Protocol", |ui| {
                let labels: Vec<&str> = PROTOCOLS.iter().map(|(label, _)| *label).collect();
                let before = PROTOCOLS[self.protocol].1.default_port();
                if segmented(ui, &mut self.protocol, &labels).changed()
                    && (self.port.trim().is_empty() || self.port.trim() == before)
                {
                    PROTOCOLS[self.protocol]
                        .1
                        .default_port()
                        .clone_into(&mut self.port);
                }
            });
            frames::group_separator(ui);
            text_row(ui, "Host", &mut self.host, "ftp.example.org", false);
            frames::group_separator(ui);
            text_row(ui, "Port", &mut self.port, "", false);
            frames::group_separator(ui);
            text_row(ui, "Path", &mut self.path, "arma3/repository", false);
            frames::group_separator(ui);
            text_row(ui, "Login", &mut self.login, ANONYMOUS, false);
            frames::group_separator(ui);
            text_row(ui, "Password", &mut self.password, "", true);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let testing = self.test.is_some();
            if ui
                .add_enabled(
                    !testing && !self.host.trim().is_empty(),
                    egui::Button::new("Test connection"),
                )
                .clicked()
            {
                self.start_test(viewer, ui.ctx());
            }
            if testing {
                ui.spinner();
            } else if let Some(result) = &self.test_result {
                let (text, color) = match result {
                    Ok(true) => (
                        "Connected; a repository is published there.".to_owned(),
                        palette.status_success,
                    ),
                    Ok(false) => (
                        "Connected; nothing is published there yet.".to_owned(),
                        palette.status_success,
                    ),
                    Err(error) => (error.clone(), palette.status_danger),
                };
                ui.add(Label::new(RichText::new(text).color(color)).wrap());
            }
        });
        if self.host.trim().is_empty() {
            ui.label(
                RichText::new("Enter the host to save the connection.")
                    .small()
                    .color(palette.text_secondary),
            );
        }
        if let Some(error) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(error).color(palette.status_danger));
        }
    }

    fn start_test(&mut self, viewer: &ViewerContext, ctx: &egui::Context) {
        let input = self.input();
        let protocol = new_protocol(
            input.protocol_type,
            &input.url,
            &input.port,
            &input.login,
            &input.password,
            input.validate_ssl_certificate,
        )
        .ok_or_else(|| "This protocol is not supported.".to_owned())
        .and_then(|protocol| {
            check_protocol(&protocol)
                .map(|()| protocol)
                .map_err(|err| err.to_string())
        });
        let protocol = match protocol {
            Ok(protocol) => protocol,
            Err(err) => {
                self.test_result = Some(Err(err));
                return;
            }
        };
        let cx = viewer.cx().clone();
        let transfer = TransferCtx {
            cancel: self.cancel.child_token(),
            ..TransferCtx::default()
        };
        self.test_result = None;
        self.test = Some(spawn(viewer, ctx, async move {
            admin::test_upload_connection(&cx, &protocol, &transfer).await
        }));
    }
}

/// `host/some/path` as `("host", "some/path")`.
fn split_url(url: &str) -> (String, String) {
    let url = strip_scheme(url);
    match url.split_once('/') {
        Some((host, path)) => (host.to_owned(), path.trim_matches('/').to_owned()),
        None => (url.to_owned(), String::new()),
    }
}
