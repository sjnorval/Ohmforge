//! Ohmforge desktop application.
//!
//! Without the `gui` feature this prints a notice; build with
//! `cargo run -p of-app --features gui` for the egui window.

#[cfg(feature = "gui")]
fn main() -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([950.0, 640.0])
        .with_min_inner_size([560.0, 570.0]);
    // Window and taskbar icon (assets/icon-192.png); skipped if it fails to decode.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon-192.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Ohmforge",
        options,
        Box::new(|cc| {
            use eframe::egui::{Color32, Theme, ThemePreference, Visuals};

            cc.egui_ctx.set_theme(ThemePreference::Dark);
            cc.egui_ctx.set_visuals_of(
                Theme::Dark,
                Visuals {
                    weak_text_color: Some(Color32::from_rgb(163, 173, 186)),
                    ..Visuals::dark()
                },
            );
            Ok(Box::new(App(of_ui::BlinkDemo::new())))
        }),
    )
}

#[cfg(feature = "gui")]
struct App(of_ui::BlinkDemo);

#[cfg(feature = "gui")]
impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let frame = eframe::egui::Frame::central_panel(ui.style()).inner_margin(eframe::egui::Margin::same(20));
        eframe::egui::CentralPanel::default()
            .frame(frame)
            .show(ui, |ui| self.0.ui(ui));
    }
}

#[cfg(not(feature = "gui"))]
fn main() {
    println!("Ohmforge desktop app. Rebuild with `--features gui` for the LED blink demo.");
}
