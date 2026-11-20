//! Shared colours and panel chrome for the desktop demos.

use egui::{Color32, Frame, Margin};

pub(crate) const TEAL: Color32 = Color32::from_rgb(99, 222, 199);
pub(crate) const LED_RED: Color32 = Color32::from_rgb(255, 93, 97);
pub(crate) const PANEL: Color32 = Color32::from_rgb(22, 26, 32);

pub(crate) fn panel() -> Frame {
    Frame::new().fill(PANEL).corner_radius(8).inner_margin(Margin::same(14))
}
