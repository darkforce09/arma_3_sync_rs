//! The Add repository sheet, also the Edit connection sheet: an auto-config link fills every
//! field; the connection, the login and the proxy can be changed by hand.

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc;

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::{ProtocolInput, ServiceError};
use a3s_sync::EventError;
use a3s_transport::{CancellationToken, TransferCtx};
use a3s_types::local::protocol::{check_protocol, new_protocol};
use a3s_types::{AutoConfig, Protocol, ProtocolType};
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::{ROW_HEIGHT, button, form_row, segmented, toggle};
use a3s_viewer_context::repositories::{self, ConnectionSummary};
use a3s_viewer_context::{Change, Command, ViewerContext, sync};
use egui::{Align, Label, Layout, RichText, TextEdit, Ui, vec2};

use super::{SheetOutcome, Shown};
use crate::picker::FolderPicker;
use crate::shell::Screen;
use crate::toasts::Toaster;

/// The protocols of the segmented control, in its order.
const PROTOCOLS: [(&str, ProtocolType); 6] = [
    ("HTTP", ProtocolType::Http),
    ("HTTPS", ProtocolType::Https),
    ("FTP", ProtocolType::Ftp),
    ("FTPS", ProtocolType::Ftps),
    ("SFTP", ProtocolType::Sftp),
    ("WebDAV", ProtocolType::HttpsWebdav),
];

/// The proxy protocols, in the order of their segmented control.
const PROXY_PROTOCOLS: [(&str, ProtocolType); 3] = [
    ("HTTP", ProtocolType::Http),
    ("SOCKS4", ProtocolType::Socks4),
    ("SOCKS5", ProtocolType::Socks5),
];

/// The login of an anonymous connection.
const ANONYMOUS: &str = "anonymous";

/// Width of the text fields.
const FIELD_WIDTH: f32 = 260.0;

/// The fields of a connection, as in [`ProtocolInput`].
pub(crate) fn protocol_input(protocol: &Protocol) -> ProtocolInput {
    ProtocolInput {
        protocol_type: protocol.protocol_type,
        url: protocol.url.clone(),
        port: protocol.port.clone(),
        login: protocol.login.clone().unwrap_or_default(),
        password: protocol.password.clone().unwrap_or_default(),
        validate_ssl_certificate: protocol.validate_ssl_certificate,
    }
}

/// Adding a repository, or editing the one named here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Add,
    Edit(String),
}

/// The proxy fields; the proxy serves every repository.
#[derive(Clone, Debug, Default)]
struct ProxyFields {
    enabled: bool,
    protocol: usize,
    host: String,
    port: String,
    /// A proxy was stored; it is written back only when a field changed.
    changed: bool,
}

/// The sheet's state.
pub(crate) struct RepositoryForm {
    mode: Mode,
    autoconfig_url: String,
    name: String,
    protocol: usize,
    secure_webdav: bool,
    address: String,
    port: String,
    anonymous: bool,
    login: String,
    password: String,
    validate_ssl: bool,
    proxy: ProxyFields,
    folder: Option<PathBuf>,
    picker: FolderPicker,
    fetch: Option<mpsc::Receiver<Result<AutoConfig, EventError>>>,
    fetch_error: Option<String>,
    test: Option<mpsc::Receiver<Result<ConnectionSummary, EventError>>>,
    test_result: Option<Result<ConnectionSummary, String>>,
    error: Option<String>,

    /// Cancels the fetch and the test when the sheet closes.
    cancel: CancellationToken,
}

impl Drop for RepositoryForm {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl RepositoryForm {
    /// The sheet for a new repository; the download folder starts as the active profile's
    /// first addon folder.
    pub(crate) fn add(viewer: &ViewerContext) -> Self {
        let state = viewer.read();
        let folder = state
            .client
            .addon_search_directories()
            .first()
            .map(PathBuf::from);
        let proxy = proxy_fields(state.client.proxy());
        drop(state);
        Self {
            mode: Mode::Add,
            autoconfig_url: String::new(),
            name: String::new(),
            protocol: 1,
            secure_webdav: true,
            address: String::new(),
            port: ProtocolType::Https.default_port().to_owned(),
            anonymous: true,
            login: String::new(),
            password: String::new(),
            validate_ssl: true,
            proxy,
            folder,
            picker: FolderPicker::default(),
            fetch: None,
            fetch_error: None,
            test: None,
            test_result: None,
            error: None,
            cancel: CancellationToken::new(),
        }
    }

    /// The sheet editing the connection of `name`; `None` when it does not exist.
    pub(crate) fn edit(viewer: &ViewerContext, name: &str) -> Option<Self> {
        let mut form = Self::add(viewer);
        let state = viewer.read();
        let repository = state.client.repository(name).ok()?;
        form.mode = Mode::Edit(name.to_owned());
        form.name = name.to_owned();
        form.fill_protocol(&repository.protocol);
        form.folder = repository
            .default_download_location
            .as_deref()
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from);
        Some(form)
    }

    /// The mode, for the sheet kind.
    pub(crate) fn mode(&self) -> &Mode {
        &self.mode
    }

    fn fill_protocol(&mut self, protocol: &Protocol) {
        let protocol_type = protocol.protocol_type;
        self.secure_webdav = protocol_type != ProtocolType::HttpWebdav;
        let shown = if protocol_type == ProtocolType::HttpWebdav {
            ProtocolType::HttpsWebdav
        } else {
            protocol_type
        };
        self.protocol = PROTOCOLS.iter().position(|(_, t)| *t == shown).unwrap_or(1);
        self.address.clone_from(&protocol.url);
        self.port.clone_from(&protocol.port);
        let login = protocol.login.clone().unwrap_or_default();
        self.anonymous = login.is_empty() || login == ANONYMOUS;
        self.login = if self.anonymous { String::new() } else { login };
        self.password = protocol.password.clone().unwrap_or_default();
        self.validate_ssl = protocol.validate_ssl_certificate;
    }

    fn fill_autoconfig(&mut self, autoconfig: &AutoConfig) {
        if let Some(name) = autoconfig
            .repository_name
            .as_deref()
            .filter(|n| !n.is_empty())
        {
            name.clone_into(&mut self.name);
        }
        if let Some(protocol) = &autoconfig.protocol {
            self.fill_protocol(protocol);
        }
    }

    fn protocol_type(&self) -> ProtocolType {
        match PROTOCOLS[self.protocol].1 {
            ProtocolType::HttpsWebdav if !self.secure_webdav => ProtocolType::HttpWebdav,
            other => other,
        }
    }

    fn input(&self) -> ProtocolInput {
        let (login, password) = if self.anonymous {
            (ANONYMOUS.to_owned(), String::new())
        } else {
            (self.login.trim().to_owned(), self.password.clone())
        };
        ProtocolInput {
            protocol_type: self.protocol_type(),
            url: strip_scheme(self.address.trim()).to_owned(),
            port: self.port.trim().to_owned(),
            login,
            password,
            validate_ssl_certificate: self.validate_ssl,
        }
    }

    fn checked_protocol(&self) -> Result<Protocol, String> {
        let input = self.input();
        let protocol = new_protocol(
            input.protocol_type,
            &input.url,
            &input.port,
            &input.login,
            &input.password,
            input.validate_ssl_certificate,
        )
        .ok_or_else(|| "This protocol is not supported.".to_owned())?;
        check_protocol(&protocol).map_err(|err| err.to_string())?;
        Ok(protocol)
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        self.poll();
        let (title, ok) = match &self.mode {
            Mode::Add => ("Add repository", "Add"),
            Mode::Edit(_) => ("Edit connection", "Save"),
        };
        let ready = !self.name.trim().is_empty() && !self.address.trim().is_empty();
        let action = Dialog::new("a3s_viewer::sheet::repository", title)
            .ok_label(ok)
            .ok_enabled(ready && self.fetch.is_none())
            .default_button(Some(DialogButton::Ok))
            .show(ctx, |ui| self.body(ui, viewer));
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => match self.save(viewer) {
                Ok(name) => {
                    let text = match &self.mode {
                        Mode::Add => format!("Added {name}"),
                        Mode::Edit(_) => format!("Saved {name}"),
                    };
                    toasts.success(text);
                    Shown::Closed(Some(SheetOutcome::Navigate(Screen::Repository(name))))
                }
                Err(err) => {
                    self.error = Some(err);
                    Shown::Open
                }
            },
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }

    /// Takes the results of the fetch, the test and the folder dialog once they arrived.
    fn poll(&mut self) {
        if let Some(folder) = self.picker.poll() {
            self.folder = Some(folder);
        }
        if let Some(result) = take(&mut self.fetch) {
            match result {
                Ok(autoconfig) => {
                    self.fill_autoconfig(&autoconfig);
                    self.fetch_error = None;
                }
                Err(err) => self.fetch_error = Some(err.message),
            }
        }
        if let Some(result) = take(&mut self.test) {
            self.test_result = Some(result.map_err(|err| err.message));
        }
    }

    fn body(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let ctx = ui.ctx().clone();
        let palette = theme::palette(&ctx);
        ui.set_width(480.0);
        ui.label("Paste the auto-config link your group shared; it fills in everything else.");
        ui.add_space(6.0);
        frames::group(ui, |ui| {
            let changed = text_row(
                ui,
                "Auto-config link",
                &mut self.autoconfig_url,
                "https://…/.a3s/autoconfig",
                false,
            );
            if changed && a3s_types::autoconfig_url::parse_import_url(&self.autoconfig_url).is_ok()
            {
                self.start_fetch(viewer, &ctx);
            }
        });
        if self.fetch.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the auto-config…");
            });
        } else if let Some(error) = &self.fetch_error {
            ui.label(RichText::new(error).color(palette.status_danger));
        }
        ui.add_space(8.0);
        frames::group(ui, |ui| {
            text_row(ui, "Name", &mut self.name, "", false);
            frames::group_separator(ui);
            form_row(ui, "Protocol", |ui| {
                let labels: Vec<&str> = PROTOCOLS.iter().map(|(label, _)| *label).collect();
                let before = self.protocol;
                if segmented(ui, &mut self.protocol, &labels).changed() {
                    let old_default = PROTOCOLS[before].1.default_port();
                    if self.port.trim().is_empty() || self.port.trim() == old_default {
                        self.port = self.protocol_type().default_port().to_owned();
                    }
                }
            });
            frames::group_separator(ui);
            if text_row(ui, "Address", &mut self.address, "host/path", false) {
                self.take_scheme();
            }
            frames::group_separator(ui);
            text_row(ui, "Port", &mut self.port, "", false);
            frames::group_separator(ui);
            form_row(ui, "Anonymous", |ui| toggle(ui, &mut self.anonymous));
            frames::group_separator(ui);
            form_row(ui, "Download folder", |ui| {
                if button(ui, "Choose…").clicked() {
                    self.picker
                        .open(&ctx, "Download folder", self.folder.as_deref());
                }
                let text = self
                    .folder
                    .as_deref()
                    .map_or_else(|| "None".to_owned(), |folder| folder.display().to_string());
                ui.add(Label::new(text).truncate());
            });
        });
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Advanced")
            .id_salt("a3s_viewer::sheet::repository::advanced")
            .default_open(false)
            .show(ui, |ui| self.advanced(ui));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let testing = self.test.is_some();
            if ui
                .add_enabled(!testing, egui::Button::new("Test connection"))
                .clicked()
            {
                self.start_test(viewer, &ctx);
            }
            if testing {
                ui.spinner();
            } else if let Some(result) = &self.test_result {
                let (text, color) = match result {
                    Ok(summary) => (summary_text(summary), palette.status_success),
                    Err(error) => (error.clone(), palette.status_danger),
                };
                ui.add(Label::new(RichText::new(text).color(color)).wrap());
            }
        });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(palette.status_danger));
        }
    }

    fn advanced(&mut self, ui: &mut Ui) {
        frames::group(ui, |ui| {
            ui.add_enabled_ui(!self.anonymous, |ui| {
                text_row(ui, "Login", &mut self.login, "", false);
                frames::group_separator(ui);
                text_row(ui, "Password", &mut self.password, "", true);
            });
            frames::group_separator(ui);
            form_row(ui, "Validate SSL certificate", |ui| {
                toggle(ui, &mut self.validate_ssl)
            });
            if PROTOCOLS[self.protocol].1 == ProtocolType::HttpsWebdav {
                frames::group_separator(ui);
                form_row(ui, "WebDAV over HTTPS", |ui| {
                    toggle(ui, &mut self.secure_webdav)
                });
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Proxy (all repositories)").small());
        frames::group(ui, |ui| {
            let proxy = &mut self.proxy;
            let mut changed = form_row(ui, "Use a proxy", |ui| toggle(ui, &mut proxy.enabled))
                .inner
                .changed();
            frames::group_separator(ui);
            changed |= form_row(ui, "Type", |ui| {
                let labels: Vec<&str> = PROXY_PROTOCOLS.iter().map(|(label, _)| *label).collect();
                segmented(ui, &mut proxy.protocol, &labels)
            })
            .inner
            .changed();
            frames::group_separator(ui);
            changed |= text_row(ui, "Proxy host", &mut proxy.host, "", false);
            frames::group_separator(ui);
            changed |= text_row(ui, "Proxy port", &mut proxy.port, "", false);
            proxy.changed |= changed;
        });
    }

    /// Moves a scheme typed into the address into the protocol control.
    fn take_scheme(&mut self) {
        let address = self.address.trim();
        let found = PROTOCOLS.iter().position(|(_, t)| {
            address
                .get(..t.prompt().len())
                .is_some_and(|head| head.eq_ignore_ascii_case(t.prompt()))
        });
        if let Some(index) = found {
            let rest = strip_scheme(address).to_owned();
            self.protocol = index;
            self.port = self.protocol_type().default_port().to_owned();
            self.address = rest;
        }
    }

    fn start_fetch(&mut self, viewer: &ViewerContext, ctx: &egui::Context) {
        let url = self.autoconfig_url.trim().to_owned();
        let cx = viewer.cx().clone();
        let transfer = self.transfer_ctx();
        self.fetch_error = None;
        self.fetch = Some(spawn(viewer, ctx, async move {
            repositories::fetch_autoconfig(&cx, &url, &transfer).await
        }));
    }

    fn start_test(&mut self, viewer: &ViewerContext, ctx: &egui::Context) {
        let protocol = match self.checked_protocol() {
            Ok(protocol) => protocol,
            Err(err) => {
                self.test_result = Some(Err(err));
                return;
            }
        };
        let name = self.name.trim().to_owned();
        let cx = viewer.cx().clone();
        let transfer = self.transfer_ctx();
        self.test_result = None;
        self.test = Some(spawn(viewer, ctx, async move {
            repositories::test_connection(&cx, &name, &protocol, &transfer).await
        }));
    }

    fn transfer_ctx(&self) -> TransferCtx {
        TransferCtx {
            cancel: self.cancel.child_token(),
            ..TransferCtx::default()
        }
    }

    /// Stores the repository (and the proxy when it changed); returns its name.
    fn save(&self, viewer: &ViewerContext) -> Result<String, String> {
        let name = self.name.trim().to_owned();
        let input = self.input();
        let folder = self.folder.as_deref().map(|f| f.display().to_string());
        let stored = match &self.mode {
            Mode::Add => viewer.mutate(Some(Change::Repository), |state| {
                state.client.create_repository(&name, &input)?;
                state
                    .client
                    .set_default_download_location(&name, None, folder.clone())
            }),
            Mode::Edit(original) => viewer
                .mutate(Some(Change::Repository), |state| {
                    state.client.set_repository_protocol(original, &input)?;
                    state
                        .client
                        .set_default_download_location(original, None, folder.clone())?;
                    sync::forget(state, original);
                    Ok::<(), ServiceError>(())
                })
                .and_then(|()| {
                    if *original == name {
                        Ok(())
                    } else {
                        viewer.rename_repository(original, &name)
                    }
                }),
        };
        stored.map_err(|err| err.user_message())?;
        if self.proxy.changed {
            self.save_proxy(viewer)?;
        }
        let check = Command::Repositories(repositories::Command::Check {
            names: vec![name.clone()],
        });
        if let Err(err) = viewer.send(check) {
            tracing::warn!("Cannot check repository {name}: {err}");
        }
        Ok(name)
    }

    fn save_proxy(&self, viewer: &ViewerContext) -> Result<(), String> {
        let proxy = &self.proxy;
        let saved = if proxy.host.trim().is_empty() {
            viewer.mutate(None, |state| state.client.set_proxy(None, false))
        } else {
            let input = ProtocolInput {
                protocol_type: PROXY_PROTOCOLS[proxy.protocol].1,
                url: proxy.host.trim().to_owned(),
                port: proxy.port.trim().to_owned(),
                login: ANONYMOUS.to_owned(),
                password: String::new(),
                validate_ssl_certificate: false,
            };
            viewer.mutate(None, |state| {
                state.client.set_proxy(Some(&input), proxy.enabled)
            })
        };
        saved.map_err(|err| err.user_message())
    }
}

fn proxy_fields(proxy: &a3s_types::local::Proxy) -> ProxyFields {
    let Some(protocol) = &proxy.proxy_protocol else {
        return ProxyFields::default();
    };
    ProxyFields {
        enabled: proxy.enable_proxy,
        protocol: PROXY_PROTOCOLS
            .iter()
            .position(|(_, t)| *t == protocol.protocol_type)
            .unwrap_or(0),
        host: protocol.url.clone(),
        port: protocol.port.clone(),
        changed: false,
    }
}

fn summary_text(summary: &ConnectionSummary) -> String {
    let mut text = match summary.revision {
        Some(revision) => format!("Connected: revision {revision}"),
        None => "Connected".to_owned(),
    };
    if let Some(mods) = summary.mods {
        text.push_str(&format!(
            ", {}",
            crate::screens::count_text(mods, "mod", "mods")
        ));
    }
    text
}

/// `url` without a leading `scheme://`.
pub(crate) fn strip_scheme(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, rest)| rest)
}

/// A form row with a labelled text field; returns whether the text changed.
pub(crate) fn text_row(
    ui: &mut Ui,
    label: &str,
    text: &mut String,
    hint: &str,
    password: bool,
) -> bool {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            let label = ui.label(label);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    TextEdit::singleline(text)
                        .hint_text(hint)
                        .password(password)
                        .desired_width(FIELD_WIDTH),
                )
                .labelled_by(label.id)
                .changed()
            })
            .inner
        },
    )
    .inner
}

/// Runs `work` on the context's runtime; the receiver gets its result and the window repaints.
fn spawn<T: Send + 'static>(
    viewer: &ViewerContext,
    ctx: &egui::Context,
    work: impl Future<Output = T> + Send + 'static,
) -> mpsc::Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    viewer.handle().spawn(async move {
        sender.send(work.await).ok();
        ctx.request_repaint();
    });
    receiver
}

/// The value `receiver` delivered, once; a closed channel ends the wait.
fn take<T>(receiver: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    let result = match receiver.as_ref()?.try_recv() {
        Ok(value) => Some(value),
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(mpsc::TryRecvError::Disconnected) => None,
    };
    *receiver = None;
    result
}
