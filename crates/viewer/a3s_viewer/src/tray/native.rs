//! The Windows and macOS tray through `tray-icon`. It must be created on the thread that runs
//! the event loop, so it is spawned from the eframe creation callback.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{ICON_SIZE, MENU};
use crate::requests::{RequestSender, WindowRequest};

/// The tray icon. Only held: dropping the handle removes the icon from the tray.
pub(super) struct Icon {
    _icon: TrayIcon,
}

pub(super) fn spawn(requests: RequestSender, rgba: &[u8]) -> Result<Icon, String> {
    let menu = Menu::new();
    let mut entries = Vec::with_capacity(MENU.len());
    for (label, request) in MENU {
        let item = MenuItem::new(label, true, None);
        menu.append(&item).map_err(|err| err.to_string())?;
        entries.push((item.id().clone(), request));
    }
    let menu_requests = requests.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some((_, request)) = entries.iter().find(|(id, _)| *id == event.id) {
            menu_requests.send(*request);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } = event
        {
            requests.send(WindowRequest::Show);
        }
    }));
    let icon = tray_icon::Icon::from_rgba(rgba.to_vec(), ICON_SIZE, ICON_SIZE)
        .map_err(|err| err.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Arma3Sync")
        .with_icon(icon)
        .build()
        .map(|icon| Icon { _icon: icon })
        .map_err(|err| err.to_string())
}
