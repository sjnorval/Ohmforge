//! Ohmforge desktop application.
//!
//! Without the `gui` feature this prints a notice; build with
//! `cargo run -p of-app --features gui` for the egui window.
//! Pass `--no-splash` or set `OF_NO_SPLASH=1` to skip the load screen for this run.
//! A persistent "Show startup animation" setting also controls the load screen;
//! the flags override it without changing the saved preference.
//! Every demo is built from its circuit file in `circuits/demos`.

#[cfg(feature = "gui")]
fn main() -> eframe::Result<()> {
    let cli_no_splash = std::env::args().any(|arg| arg == "--no-splash");
    let prefs = of_ui::prefs::load();
    let no_splash = !of_ui::prefs::should_play_splash(cli_no_splash, prefs.show_startup_animation);
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([980.0, 720.0])
        .with_min_inner_size([560.0, 620.0])
        // Open maximised; restoring the window falls back to the inner size above.
        .with_maximized(true);
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
        Box::new(move |cc| {
            use eframe::egui::{Color32, Theme, ThemePreference, Visuals};

            cc.egui_ctx.set_theme(ThemePreference::Dark);
            cc.egui_ctx.set_visuals_of(
                Theme::Dark,
                Visuals {
                    weak_text_color: Some(Color32::from_rgb(163, 173, 186)),
                    ..Visuals::dark()
                },
            );
            Ok(Box::new(App::new(no_splash)))
        }),
    )
}

#[cfg(feature = "gui")]
struct App {
    splash: Option<of_ui::splash::SplashView>,
    demo: Option<of_ui::OhmforgeDemo>,
}

#[cfg(feature = "gui")]
impl App {
    fn new(no_splash: bool) -> Self {
        let demo = of_ui::OhmforgeDemo::new;
        if no_splash {
            return Self {
                splash: None,
                demo: Some(demo()),
            };
        }
        match of_ui::splash::SplashView::load() {
            Ok(splash) => Self {
                splash: Some(splash),
                demo: None,
            },
            Err(err) => {
                eprintln!("Ohmforge splash: {err}");
                Self {
                    splash: None,
                    demo: Some(demo()),
                }
            }
        }
    }
}

#[cfg(feature = "gui")]
impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let start_demo = self.demo.is_none() && self.splash.as_ref().is_some_and(|splash| splash.has_painted());
        if start_demo {
            self.demo = Some(of_ui::OhmforgeDemo::new());
        }

        let show_demo = self.splash.as_ref().is_none_or(|splash| !splash.is_covering());
        if show_demo {
            if self.demo.is_none() {
                self.demo = Some(of_ui::OhmforgeDemo::new());
            }
            let frame = eframe::egui::Frame::central_panel(ui.style()).inner_margin(eframe::egui::Margin::same(20));
            eframe::egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                if let Some(demo) = &mut self.demo {
                    demo.ui(ui);
                }
            });
        } else {
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::NONE.fill(eframe::egui::Color32::BLACK))
                .show(ui, |_| {});
        }

        if let Some(splash) = &mut self.splash {
            splash.show(ui.ctx(), self.demo.is_some());
            if !splash.is_active() {
                self.splash = None;
            }
        }
    }
}

#[cfg(not(feature = "gui"))]
fn main() {
    println!("Ohmforge desktop app. Rebuild with `--features gui` for the LED blink, pot dimmer and LED fade demos.");
}
