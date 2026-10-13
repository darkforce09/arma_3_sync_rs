//! The Linux tray: a `StatusNotifierItem` over D-Bus through `ksni` (KDE, and GNOME with the
//! `AppIndicator` extension).

use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::StandardItem;

use super::{ICON_SIZE, MENU};
use crate::requests::{RequestSender, WindowRequest};

struct StatusNotifier {
    requests: RequestSender,

    /// ARGB32 in network byte order.
    icon: Vec<u8>,
}

impl ksni::Tray for StatusNotifier {
    fn id(&self) -> String {
        "arma3sync".to_owned()
    }

    fn title(&self) -> String {
        "Arma3Sync".to_owned()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = ICON_SIZE as i32;
        vec![ksni::Icon {
            width: size,
            height: size,
            data: self.icon.clone(),
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.requests.send(WindowRequest::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        MENU.iter()
            .map(|&(label, request)| {
                StandardItem {
                    label: label.to_owned(),
                    activate: Box::new(move |tray: &mut Self| tray.requests.send(request)),
                    ..StandardItem::default()
                }
                .into()
            })
            .collect()
    }
}

/// The tray service; dropping it removes the icon.
pub(super) struct Icon(Handle<StatusNotifier>);

impl Drop for Icon {
    fn drop(&mut self) {
        // The service thread ends on its own; waiting for it is not needed.
        drop(self.0.shutdown());
    }
}

pub(super) fn spawn(requests: RequestSender, rgba: &[u8]) -> Result<Icon, ksni::Error> {
    let icon = rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
        .collect();
    StatusNotifier { requests, icon }.spawn().map(Icon)
}
