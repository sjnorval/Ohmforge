//! The routed schematic of each desktop demo, built from its embedded circuit
//! file (`wiring::from_circuit`). The route goldens in `wiring` pin the look.

use crate::kind::DemoKind;
use crate::session_sim;
use crate::wiring::{self, Schematic};

/// The schematic to paint for `kind`. `position` is the dimmer's wiper
/// (0 = pin 1) and is ignored by the other demos.
pub(crate) fn board(kind: DemoKind, position: Option<f64>) -> Schematic {
    file_board(kind, position).unwrap_or_else(|e| {
        // Unreachable for the shipped files (tested).
        eprintln!("Ohmforge: {} schematic: {e}", session_sim::demo_file(kind).path);
        Schematic::default()
    })
}

/// The routed schematic of `kind`'s embedded circuit file, with the live
/// wiper `position` on the dimmer.
pub(crate) fn file_board(kind: DemoKind, position: Option<f64>) -> Result<Schematic, String> {
    let file = session_sim::parsed(kind)?;
    let live = move |reference: &str, param: &str| {
        (kind == DemoKind::Dimmer && reference.eq_ignore_ascii_case("RV1") && param == "position")
            .then_some(position)
            .flatten()
    };
    wiring::from_circuit(file, &live)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [DemoKind; 5] = [
        DemoKind::Blink,
        DemoKind::Dimmer,
        DemoKind::Fade,
        DemoKind::ButtonPoll,
        DemoKind::ButtonIrq,
    ];

    /// What the painters get is the file's routed schematic: byte for byte
    /// the route golden (the dimmer at mid-travel), and clean at every wiper
    /// position.
    #[test]
    fn the_painters_get_the_route_goldens() {
        let golden = |kind| match kind {
            DemoKind::Blink => include_str!("wiring/goldens/blink.routes.txt"),
            DemoKind::Dimmer => include_str!("wiring/goldens/dimmer.routes.txt"),
            DemoKind::Fade => include_str!("wiring/goldens/fade.routes.txt"),
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => include_str!("wiring/goldens/button.routes.txt"),
        };
        for kind in KINDS {
            let mid = kind.has_pot().then_some(0.5);
            assert_eq!(wiring::serialize(&board(kind, mid)), golden(kind), "{kind:?}");
            for position in [None, Some(0.0), Some(0.3), Some(0.5), Some(1.0)] {
                let b = board(kind, position);
                assert_eq!(b, file_board(kind, position).unwrap(), "{kind:?} at {position:?}");
                assert!(wiring::check(&b).is_empty(), "{kind:?} at {position:?}");
            }
        }
    }
}
