//! The bench PSU on the desktop: labels and warnings. The settings and status
//! types are `of_components::psu`'s (E5d folded the duplicates).

pub use of_components::psu::{RATED_MAX_V, RATED_MIN_V, SupplyMode, SupplySettings, SupplyStatus};

/// Short warning when the supply is outside the modelled chip's ratings.
pub fn rating_warning(status: &SupplyStatus) -> Option<&'static str> {
    status
        .outside_rated
        .then_some("outside ESP32-C3 rated supply (3.0–3.6 V); brownout/undervoltage is not modelled")
}

/// Mode label for the live readout.
pub fn mode_label(mode: SupplyMode) -> &'static str {
    match mode {
        SupplyMode::Off => "OFF",
        SupplyMode::Cv => "CV",
        SupplyMode::Cc => "CC",
    }
}
