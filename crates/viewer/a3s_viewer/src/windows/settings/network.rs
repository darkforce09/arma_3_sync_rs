//! Network: the proxy every connection goes through.

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::ProtocolInput;
use a3s_types::ProtocolType;
use a3s_types::local::Proxy;
use a3s_ui::frames;
use a3s_ui::widgets::popup_button;
use a3s_viewer_context::Change;
use egui::Ui;

use super::{PaneCx, note};
use crate::fields::{labelled_row, text_row};

/// The login that means "no credentials".
const ANONYMOUS: &str = "anonymous";

const KINDS: [(Option<ProtocolType>, &str); 5] = [
    (None, "None"),
    (Some(ProtocolType::Http), "HTTP"),
    (Some(ProtocolType::Https), "HTTPS"),
    (Some(ProtocolType::Socks4), "SOCKS 4"),
    (Some(ProtocolType::Socks5), "SOCKS 5"),
];

/// The proxy fields as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProxyDraft {
    kind: Option<ProtocolType>,
    host: String,
    port: String,
    login: String,
    password: String,
}

impl ProxyDraft {
    fn of(proxy: &Proxy) -> Self {
        let Some(protocol) = proxy.proxy_protocol.as_ref().filter(|_| proxy.enable_proxy) else {
            return Self::default();
        };
        let login = protocol.login.clone().unwrap_or_default();
        Self {
            kind: Some(protocol.protocol_type),
            host: protocol.url.clone(),
            port: protocol.port.clone(),
            login: if login == ANONYMOUS {
                String::new()
            } else {
                login
            },
            password: protocol.password.clone().unwrap_or_default(),
        }
    }

    fn input(&self, kind: ProtocolType) -> ProtocolInput {
        ProtocolInput {
            protocol_type: kind,
            url: self.host.trim().to_owned(),
            port: self.port.trim().to_owned(),
            login: if self.login.trim().is_empty() {
                ANONYMOUS.to_owned()
            } else {
                self.login.trim().to_owned()
            },
            password: self.password.clone(),
            validate_ssl_certificate: true,
        }
    }
}

#[derive(Default)]
pub(super) struct NetworkPane {
    /// The fields while one of them has the focus or holds a value that cannot be saved.
    draft: Option<ProxyDraft>,

    /// Why the typed proxy cannot be saved.
    problem: Option<String>,
}

impl NetworkPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &PaneCx<'_>) {
        let stored = ProxyDraft::of(cx.viewer.read().client.proxy());
        let mut draft = self.draft.clone().unwrap_or_else(|| stored.clone());
        let mut focused = false;
        let mut edited = false;

        frames::group(ui, |ui| {
            edited |= labelled_row(ui, "Proxy", |ui| {
                popup_button(
                    ui,
                    "a3s_viewer::settings::proxy_kind",
                    &mut draft.kind,
                    &KINDS,
                )
            })
            .changed();
            if draft.kind.is_some() {
                let mut field = |ui: &mut Ui, label: &str, text: &mut String, password: bool| {
                    let response = text_row(ui, label, text, password);
                    focused |= response.has_focus();
                    edited |= response.lost_focus();
                };
                frames::group_separator(ui);
                field(ui, "Server", &mut draft.host, false);
                frames::group_separator(ui);
                field(ui, "Port", &mut draft.port, false);
                frames::group_separator(ui);
                field(ui, "User name", &mut draft.login, false);
                frames::group_separator(ui);
                field(ui, "Password", &mut draft.password, true);
            }
        });
        if let Some(problem) = &self.problem {
            ui.add_space(4.0);
            note(ui, problem.clone());
        }

        if edited && draft != stored {
            self.save(cx, &draft);
        } else if edited {
            self.problem = None;
        }
        self.draft = (focused || self.problem.is_some()).then_some(draft);
    }

    /// Writes the proxy; a value that cannot be saved stays in the fields with the reason.
    fn save(&mut self, cx: &PaneCx<'_>, draft: &ProxyDraft) {
        let saved = cx
            .viewer
            .mutate(Some(Change::Online), |state| match draft.kind {
                None => state.client.set_proxy(None, false),
                Some(kind) => state.client.set_proxy(Some(&draft.input(kind)), true),
            });
        self.problem = saved.err().map(|err| err.user_message());
    }
}
