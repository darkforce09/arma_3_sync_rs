//! Connection tests and autoconfig imports for the repository sheet, before anything is
//! stored.

use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_sync::{EventError, EventSender, RemoteError, RemoteMetadata};
use a3s_transport::TransferCtx;
use a3s_transport::proxy::transport_options;
use a3s_types::{AutoConfig, Protocol, SyncTreeDirectory, autoconfig_url};

use crate::checker::RemoteCheck;
use crate::tasks::Cx;

/// What a connection test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionSummary {
    /// The server revision.
    pub revision: Option<i32>,

    /// Mods in the repository, when its content tree was read.
    pub mods: Option<usize>,
}

/// Fetches the metadata of a repository at `protocol` through the context's checker, with the
/// configured proxy. Runs on the context's runtime.
pub async fn test_connection(
    cx: &Cx,
    name: &str,
    protocol: &Protocol,
    ctx: &TransferCtx,
) -> Result<ConnectionSummary, EventError> {
    let proxy = cx.state().read().client.proxy().clone();
    let options = transport_options(protocol, &proxy)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    let mut metadata = RemoteMetadata::default();
    cx.shared
        .checker
        .check(RemoteCheck {
            repository_name: name,
            protocol,
            options: &options,
            metadata: &mut metadata,
            ctx,
            events: &EventSender::noop(),
        })
        .await
        .map_err(|err| EventError::from_remote(&err))?;
    Ok(ConnectionSummary {
        revision: metadata.server_revision(),
        mods: addon_count(&metadata),
    })
}

/// Fetches and decodes the autoconfig file at `url` (`…/.a3s/autoconfig`), with the
/// configured proxy.
// Mirrors ui/repository/dialogs/connection/DescriptionPanel.java:234-262
pub async fn fetch_autoconfig(
    cx: &Cx,
    url: &str,
    ctx: &TransferCtx,
) -> Result<AutoConfig, EventError> {
    let protocol = autoconfig_url::parse_import_url(url)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    let proxy = cx.state().read().client.proxy().clone();
    let options = transport_options(&protocol, &proxy)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    a3s_sync::remote::import_autoconfig(&protocol, &options, ctx)
        .await
        .map_err(|err: RemoteError| EventError::new(err.kind(), err.user_message()))
}

/// The number of mods (folders marked as addons) in the repository's content tree, when it was
/// fetched.
pub fn addon_count(metadata: &RemoteMetadata) -> Option<usize> {
    fn count(directory: &SyncTreeDirectory) -> usize {
        directory
            .directories()
            .map(|child| if child.mark_as_addon { 1 } else { count(child) })
            .sum()
    }
    metadata.sync.as_deref().map(count)
}
