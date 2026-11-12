//! Ohmforge desktop application.
//!
//! Without the `gui` feature this prints a notice; build with
//! `cargo run -p of-app --features gui` for the egui window.

#[cfg(feature = "gui")]
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Ohmforge",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App))),
    )
}

#[cfg(feature = "gui")]
struct App;

#[cfg(feature = "gui")]
impl eframe::App for App {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Ohmforge");
            ui.label("ESP32 circuit simulator: pre-alpha scaffold.");
        });
    }
}

#[cfg(not(feature = "gui"))]
fn main() {
    println!("Ohmforge desktop app scaffold. Rebuild with `--features gui` for the window.");
}
