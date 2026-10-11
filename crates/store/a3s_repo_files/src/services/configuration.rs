//! Configuration and preferences (`service/ConfigurationService.java`,
//! `service/PreferencesService.java`).

use a3s_types::FavoriteServer;
use a3s_types::local::protocol::{check_protocol, new_protocol};
use a3s_types::local::{Configuration, ExternalApplication, Preferences, Proxy};

use super::{ClientData, FavoriteServerEdit, ProtocolInput, Result, ServiceError};

impl ClientData {
    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }

    /// Runs `f` on a copy of the configuration and, when it returns `true` ("changed"), writes
    /// the copy (favorite servers re-sorted) and keeps it. `false` writes nothing.
    ///
    /// `f` may switch the active profile only to an existing one, else
    /// [`ServiceError::ProfileNotFound`] and nothing changes.
    pub fn update_configuration(
        &mut self,
        f: impl FnOnce(&mut Configuration) -> bool,
    ) -> Result<bool> {
        self.try_update_configuration(|c| Ok(f(c)))
    }

    pub(super) fn try_update_configuration(
        &mut self,
        f: impl FnOnce(&mut Configuration) -> Result<bool>,
    ) -> Result<bool> {
        let mut next = self.configuration.clone();
        if !f(&mut next)? {
            return Ok(false);
        }
        if !self.profiles.contains_key(&next.profile_name) {
            return Err(ServiceError::ProfileNotFound(next.profile_name));
        }
        self.commit_configuration(next)?;
        Ok(true)
    }

    /// Runs `f` on the configuration and always writes it.
    fn set_configuration(&mut self, f: impl FnOnce(&mut Configuration)) -> Result<()> {
        self.update_configuration(|c| {
            f(c);
            true
        })
        .map(drop)
    }

    // Mirrors service/ConfigurationService.java:42-44
    pub fn active_profile_name(&self) -> &str {
        &self.configuration.profile_name
    }

    /// Makes `name` the active profile; it must exist.
    // Mirrors service/ConfigurationService.java:46-52
    pub fn set_active_profile(&mut self, name: &str) -> Result<()> {
        if !self.profiles.contains_key(name) {
            return Err(ServiceError::ProfileNotFound(name.to_owned()));
        }
        self.set_configuration(|c| name.clone_into(&mut c.profile_name))
    }

    // Mirrors service/ConfigurationService.java:58-64
    pub fn set_server_name(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.server_name = value)
    }

    // Mirrors service/ConfigurationService.java:70-76
    pub fn set_default_modset(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.default_modset = value)
    }

    // Mirrors service/ConfigurationService.java:82-89
    pub fn set_game_version(&mut self, value: String) -> Result<()> {
        self.set_configuration(|c| c.game_version = value)
    }

    // Mirrors service/ConfigurationService.java:95-101
    pub fn set_view_mode_tree(&mut self, value: bool) -> Result<()> {
        self.set_configuration(|c| c.view_mode_tree = value)
    }

    /// The favorite servers, sorted case-insensitively by name. Every index-based operation
    /// refers to this order.
    // Mirrors service/ConfigurationService.java:104-113
    pub fn favorite_servers(&self) -> &[FavoriteServer] {
        &self.configuration.favorite_servers
    }

    /// Adds a server (unselected; no de-duplication) and keeps the list sorted.
    // Mirrors service/ConfigurationService.java:115-123
    pub fn add_favorite_server(&mut self, mut server: FavoriteServer) -> Result<()> {
        // The DTO has no `selected` field (service/ObjectDTOtransformer.java:103-113).
        server.selected = false;
        self.set_configuration(|c| c.favorite_servers.push(server))
    }

    /// Removes the server at `index`; `false` (nothing written) when out of range.
    // Mirrors service/ConfigurationService.java:125-134
    pub fn delete_favorite_server(&mut self, index: usize) -> Result<bool> {
        self.update_configuration(|c| {
            let in_range = index < c.favorite_servers.len();
            if in_range {
                c.favorite_servers.remove(index);
            }
            in_range
        })
    }

    /// Overwrites the edited fields of the server at `index` (keeping `selected` and
    /// `repository_name`) and re-sorts; `false` (nothing written) when out of range.
    // Mirrors service/ConfigurationService.java:136-151
    pub fn edit_favorite_server(&mut self, index: usize, edit: FavoriteServerEdit) -> Result<bool> {
        self.update_configuration(|c| {
            let Some(server) = c.favorite_servers.get_mut(index) else {
                return false;
            };
            server.name = edit.name;
            server.ip_address = edit.ip_address;
            server.port = edit.port;
            server.password = edit.password;
            server.modset_name = edit.modset_name;
            true
        })
    }

    /// Replaces the external applications with `applications`, stably sorted by name.
    // Mirrors service/ConfigurationService.java:165-181
    pub fn save_external_applications(
        &mut self,
        mut applications: Vec<ExternalApplication>,
    ) -> Result<()> {
        applications.sort_by(ExternalApplication::java_cmp);
        self.set_configuration(|c| c.external_applications = applications)
    }

    /// The stored path only; detection when unset is the caller's.
    pub fn set_acre2_ts3_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.acre2_options.ts3_path = value)
    }

    pub fn set_acre2_plugin_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.acre2_options.acre2_plugin_path = value)
    }

    pub fn set_tfar_ts3_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.tfar_options.ts3_path = value)
    }

    pub fn set_tfar_plugin_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.tfar_options.tfar_plugin_path = value)
    }

    pub fn set_tfar_userconfig_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.tfar_options.tfar_userconfig_path = value)
    }

    pub fn set_rpt_path(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.rpt_options.rpt_path = value)
    }

    pub fn set_bikey_source_directory(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.bikey_extract_options.source_directory_path = value)
    }

    pub fn set_bikey_target_directory(&mut self, value: Option<String>) -> Result<()> {
        self.set_configuration(|c| c.bikey_extract_options.target_directory_path = value)
    }

    // Mirrors service/ConfigurationService.java:311-325
    pub fn set_window_size(&mut self, height: i32, width: i32) -> Result<()> {
        self.set_configuration(|c| {
            c.height = height;
            c.width = width;
        })
    }

    // Mirrors service/ConfigurationService.java:327-331
    pub fn proxy(&self) -> &Proxy {
        &self.configuration.proxy
    }

    /// Sets the proxy: `None` removes it (and disables it); otherwise the protocol is built
    /// with timeouts `"0"` and checked. On error nothing changes.
    // Mirrors service/ConfigurationService.java:333-354
    pub fn set_proxy(&mut self, input: Option<&ProtocolInput>, enable: bool) -> Result<()> {
        let Some(input) = input else {
            return self.set_configuration(|c| {
                c.proxy.proxy_protocol = None;
                c.proxy.enable_proxy = false;
            });
        };
        let mut protocol = new_protocol(
            input.protocol_type,
            &input.url,
            &input.port,
            &input.login,
            &input.password,
            input.validate_ssl_certificate,
        )
        .ok_or(ServiceError::UnsupportedProtocol(
            "Proxy protocol type error.",
        ))?;
        protocol.connection_timeout = Some("0".to_owned());
        protocol.read_timeout = Some("0".to_owned());
        check_protocol(&protocol)?;
        self.set_configuration(|c| {
            c.proxy.proxy_protocol = Some(protocol);
            c.proxy.enable_proxy = enable;
        })
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Replaces the preferences and writes them.
    // Mirrors service/PreferencesService.java:29-54
    pub fn set_preferences(&mut self, preferences: Preferences) -> Result<()> {
        self.update_preferences(|p| {
            *p = preferences;
            true
        })
        .map(drop)
    }

    /// Runs `f` on a copy of the preferences and, when it returns `true`, writes and keeps it.
    pub fn update_preferences(&mut self, f: impl FnOnce(&mut Preferences) -> bool) -> Result<bool> {
        let mut next = self.preferences.clone();
        if !f(&mut next) {
            return Ok(false);
        }
        self.store.write_preferences(&next)?;
        self.preferences = next;
        Ok(true)
    }
}
