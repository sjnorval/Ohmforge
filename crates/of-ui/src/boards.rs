//! The routed schematic of each desktop demo: the hand-written board, or with
//! `--circuit-files` the one its embedded circuit file describes (E4c). The
//! two are equal (`wiring` tests the unrouted boards and the route goldens;
//! the tests here compare the routed boards the painters receive).

use crate::kind::DemoKind;
use crate::session_sim;
use crate::wiring::{self, Schematic};

/// The schematic to paint for `kind`. `position` is the dimmer's wiper
/// (0 = pin 1) and is ignored by the other demos.
pub(crate) fn board(kind: DemoKind, position: Option<f64>, circuit_files: bool) -> Schematic {
    if circuit_files {
        match file_board(kind, position) {
            Ok(s) => return s,
            // Unreachable for the shipped files (tested); never leave the canvas empty.
            Err(e) => eprintln!("Ohmforge: {} schematic: {e}", session_sim::demo_file(kind).path),
        }
    }
    hand_wired(kind, position)
}

fn hand_wired(kind: DemoKind, position: Option<f64>) -> Schematic {
    match kind {
        DemoKind::Blink => wiring::blink_board(),
        DemoKind::Dimmer => wiring::dimmer_board_at(position.unwrap_or(0.5)),
        DemoKind::Fade => wiring::fade_board(),
        DemoKind::ButtonPoll | DemoKind::ButtonIrq => wiring::button_board(),
    }
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

    #[test]
    fn file_boards_are_the_hand_written_boards_the_painters_get() {
        for kind in KINDS {
            for position in [None, Some(0.0), Some(0.3), Some(0.5), Some(1.0)] {
                let file = file_board(kind, position).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
                let hand = hand_wired(kind, position);
                assert_eq!(
                    wiring::serialize(&file),
                    wiring::serialize(&hand),
                    "{kind:?} at {position:?}"
                );
                assert_eq!(file, hand, "{kind:?} at {position:?}");
                assert_eq!(board(kind, position, true), hand, "{kind:?} at {position:?}");
                assert!(wiring::check(&file).is_empty(), "{kind:?} at {position:?}");
            }
        }
    }
}
