//! Shows the `a3s_ui` gallery in a native window.
//!
//! Run with `cargo run -p a3s_ui --example demo`.

use a3s_ui::gallery::Gallery;

struct DemoApp {
    gallery: Gallery,
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.gallery.ui(ui));
        });
    }
}

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("a3s_ui demo")
        .with_inner_size([1000.0, 800.0])
        .with_icon(
            eframe::icon_data::from_png_bytes(a3s_ui::icon::APP_ICON_PNG)
                .expect("the embedded app icon is a valid PNG"),
        );
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "a3s_ui demo",
        options,
        Box::new(|cc| {
            a3s_ui::install(&cc.egui_ctx);
            Ok(Box::new(DemoApp {
                gallery: Gallery::default(),
            }))
        }),
    )
}
