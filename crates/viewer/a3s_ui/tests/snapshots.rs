//! Headless wgpu snapshots of the gallery and the dialogs.
//!
//! Rendered through wgpu on Mesa's lavapipe Vulkan driver. Regenerate with
//! `UPDATE_SNAPSHOTS=1`.
#![cfg(target_os = "linux")]

use a3s_ui::dialog::{Dialog, ProgressDialog};
use a3s_ui::gallery::Gallery;
use a3s_ui::theme::{self, ThemeMode};
use a3s_ui::widgets::Progress;
use egui_kittest::Harness;

fn gallery() -> Gallery {
    let mut gallery = Gallery::default();
    gallery.freeze_animations = true;
    gallery
}

fn snapshot_gallery(
    name: &str,
    mode: ThemeMode,
    size: (f32, f32),
    mut show: impl FnMut(&mut egui::Ui, &mut Gallery) + 'static,
) {
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_theme(mode.into())
        .wgpu()
        .build_ui_state(
            move |ui, gallery| {
                egui::CentralPanel::default().show(ui, |ui| show(ui, gallery));
            },
            gallery(),
        );
    a3s_ui::install(&harness.ctx);
    theme::set_mode(&harness.ctx, mode);
    harness.run();
    harness.snapshot(name);
}

#[test]
fn gallery_light() {
    snapshot_gallery(
        "gallery_light",
        ThemeMode::Light,
        (1000.0, 2400.0),
        |ui, g| {
            g.ui(ui);
        },
    );
}

#[test]
fn gallery_dark() {
    snapshot_gallery(
        "gallery_dark",
        ThemeMode::Dark,
        (1000.0, 2400.0),
        |ui, g| {
            g.ui(ui);
        },
    );
}

#[test]
fn icons_light() {
    snapshot_gallery("icons_light", ThemeMode::Light, (1000.0, 520.0), |ui, g| {
        g.icons_ui(ui);
    });
}

#[test]
fn icons_dark() {
    snapshot_gallery("icons_dark", ThemeMode::Dark, (1000.0, 520.0), |ui, g| {
        g.icons_ui(ui);
    });
}

#[test]
fn progress_light() {
    snapshot_gallery(
        "progress_light",
        ThemeMode::Light,
        (1000.0, 260.0),
        |ui, g| {
            g.progress_ui(ui);
        },
    );
}

#[test]
fn progress_dark() {
    snapshot_gallery(
        "progress_dark",
        ThemeMode::Dark,
        (1000.0, 260.0),
        |ui, g| {
            g.progress_ui(ui);
        },
    );
}

#[test]
fn check_tree_dark() {
    snapshot_gallery(
        "check_tree_dark",
        ThemeMode::Dark,
        (420.0, 360.0),
        |ui, g| {
            g.check_tree_ui(ui);
        },
    );
}

fn backdrop(ui: &mut egui::Ui) {
    ui.heading("Arma3Sync");
    for i in 0..12 {
        ui.label(format!("Main window content line {i}"));
    }
}

fn snapshot_dialog(name: &str, mode: ThemeMode, mut show: impl FnMut(&mut egui::Ui) + 'static) {
    let mut harness = Harness::builder()
        .with_size((800.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_theme(mode.into())
        .wgpu()
        .build_ui(move |ui| {
            egui::CentralPanel::default().show(ui, |ui| show(ui));
        });
    a3s_ui::install(&harness.ctx);
    theme::set_mode(&harness.ctx, mode);
    harness.run();
    harness.snapshot(name);
}

#[test]
fn dialog_modal_dark() {
    snapshot_dialog("dialog_modal_dark", ThemeMode::Dark, |ui| {
        backdrop(ui);
        Dialog::new("snapshot_modal", "Modal dialog").show(ui.ctx(), |ui| {
            ui.label("Dialog content");
        });
    });
}

#[test]
fn dialog_window_light() {
    snapshot_dialog("dialog_window_light", ThemeMode::Light, |ui| {
        backdrop(ui);
        Dialog::new("snapshot_window", "Dialog")
            .modal(false)
            .show(ui.ctx(), |ui| {
                ui.label("Dialog content");
            });
    });
}

#[test]
fn progress_dialog_dark() {
    let mut dialog = ProgressDialog::new("snapshot_progress", "Downloading files...");
    dialog.progress = Progress::of(50, 100);
    snapshot_dialog("progress_dialog_dark", ThemeMode::Dark, move |ui| {
        backdrop(ui);
        dialog.show(ui.ctx());
    });
}
