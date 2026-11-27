//! The bench PSU on the desktop: labels and warnings. The settings and status
//! types are `of_components::psu`'s (E5d folded the duplicates).

pub use of_components::psu::{SupplyMode, SupplySettings, SupplyStatus};

use of_circuit::library::RailPart;

/// `U2 ti.ne555 4.5–16 V, …`: the parts and their supply ranges.
fn part_list(rail: &[RailPart]) -> String {
    rail.iter()
        .map(|p| format!("{} {} {}\u{2013}{} V", p.reference, p.part, p.min, p.max))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Warning when the bench supply is outside the supply range of the parts
/// on its + rail (`rail`, from the loaded circuit), naming them. `None`
/// when no part on the rail states a range or the supply is inside it.
/// The text holds no live readings, so it does not change per frame.
pub fn rating_warning(status: &SupplyStatus, set: &SupplySettings, rail: &[RailPart]) -> Option<String> {
    if rail.is_empty() {
        return None;
    }
    let lo = rail.iter().map(|p| p.min).fold(f64::MIN, f64::max);
    let hi = rail.iter().map(|p| p.max).fold(f64::MAX, f64::min);
    let list = part_list(rail);
    if lo > hi {
        return Some(format!("PSU + rail: the parts on it share no supply range ({list})"));
    }
    if set.on && !(lo..=hi).contains(&set.volts) {
        return Some(format!(
            "PSU set to {:.2} V is outside the rated supply of {list}; behaviour outside the rating is not modelled",
            set.volts
        ));
    }
    status.outside_rated.then(|| {
        format!("PSU output is outside the rated supply of {list} (current limit or off); behaviour outside the rating is not modelled")
    })
}

/// Mode label for the live readout.
pub fn mode_label(mode: SupplyMode) -> &'static str {
    match mode {
        SupplyMode::Off => "OFF",
        SupplyMode::Cv => "CV",
        SupplyMode::Cc => "CC",
    }
}

#[cfg(test)]
mod tests {
    use super::{SupplySettings, SupplyStatus, rating_warning};
    use of_circuit::library::RailPart;

    fn part(reference: &str, id: &str, min: f64, max: f64) -> RailPart {
        RailPart {
            reference: reference.into(),
            part: id.into(),
            min,
            max,
        }
    }

    fn set(volts: f64) -> SupplySettings {
        SupplySettings {
            volts,
            ..SupplySettings::default()
        }
    }

    /// The warning names exactly the rail's parts and ranges; no part, no
    /// warning; inside the range, none either.
    #[test]
    fn the_warning_names_only_the_rail_parts() {
        let ne555 = [part("U2", "ti.ne555", 4.5, 16.0)];
        let ok = SupplyStatus::default();
        let w = rating_warning(&ok, &set(3.3), &ne555).expect("3.3 V is below 4.5 V");
        assert!(w.contains("U2 ti.ne555 4.5\u{2013}16 V"), "{w}");
        assert!(!w.to_lowercase().contains("esp32"), "{w}");
        assert!(w.contains("3.30 V"), "{w}");
        assert_eq!(rating_warning(&ok, &set(5.0), &ne555), None);
        assert_eq!(rating_warning(&ok, &set(30.0), &[]), None);
        let off = SupplySettings { on: false, ..set(0.0) };
        assert_eq!(rating_warning(&ok, &off, &ne555), None);
        let sag = SupplyStatus {
            outside_rated: true,
            ..ok
        };
        assert!(rating_warning(&sag, &set(5.0), &ne555).unwrap().contains("U2 ti.ne555"));
        let mixed = [part("U1", "espressif.esp32-c3", 3.0, 3.6), ne555[0].clone()];
        let w = rating_warning(&ok, &set(3.3), &mixed).unwrap();
        assert!(
            w.contains("share no supply range") && w.contains("U1 espressif.esp32-c3"),
            "{w}"
        );
    }
}
