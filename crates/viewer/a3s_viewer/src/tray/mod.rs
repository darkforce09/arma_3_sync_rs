//! The tray icon with its menu: Show, Hide, Check now, Quit. A click on the icon shows the
//! window. `StatusNotifierItem` over D-Bus (`ksni`) on Linux, `tray-icon` on Windows and macOS.

#[cfg(target_os = "linux")]
#[path = "status_notifier.rs"]
mod platform;

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[path = "native.rs"]
mod platform;

use crate::requests::{RequestSender, WindowRequest};

/// The menu entries, in order.
const MENU: [(&str, WindowRequest); 4] = [
    ("Show Arma3Sync", WindowRequest::Show),
    ("Hide", WindowRequest::Hide),
    ("Check now", WindowRequest::CheckNow),
    ("Quit", WindowRequest::Quit),
];

/// The icon side length in pixels.
const ICON_SIZE: u32 = 64;

/// A running tray icon; it is removed when dropped.
pub(crate) struct Tray {
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    _icon: platform::Icon,
}

impl Tray {
    /// Puts the icon into the tray. `None` when the desktop has no tray (the window then never
    /// hides into it). Windows and macOS need the event loop's thread.
    pub(crate) fn spawn(requests: &RequestSender) -> Option<Self> {
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        {
            let rgba = icon_rgba()?;
            platform::spawn(requests.clone(), &rgba)
                .map_err(|err| tracing::warn!("No tray icon: {err}"))
                .ok()
                .map(|icon| Self { _icon: icon })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = requests;
            None
        }
    }
}

/// The application icon scaled to [`ICON_SIZE`], as RGBA rows.
fn icon_rgba() -> Option<Vec<u8>> {
    let icon = eframe::icon_data::from_png_bytes(a3s_ui::icon::APP_ICON_PNG)
        .map_err(|err| tracing::warn!("Cannot decode the application icon: {err}"))
        .ok()?;
    let (width, height) = (icon.width as usize, icon.height as usize);
    let size = ICON_SIZE as usize;
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            // Nearest neighbour: the icon is a flat logo, smoothing adds nothing.
            let source = ((y * height / size) * width + x * width / size) * 4;
            out.extend_from_slice(&icon.rgba[source..source + 4]);
        }
    }
    Some(out)
}
