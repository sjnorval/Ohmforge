//! Virtual voltmeters and ammeters on the live schematic. Placement and
//! readings are `of_instruments::meter` (E5c): the live worker samples each
//! placed [`Multimeter`] and publishes its reading; this module keeps the
//! schematic-side names.

use of_instruments::Multimeter;

/// A voltmeter or a non-disturbing ammeter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterKind {
    /// `V(A) − V(B)`.
    Volt,
    /// Current through the single part that joins A and B, from A toward B.
    Amp,
}

impl MeterKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Volt => "Voltmeter",
            Self::Amp => "Ammeter",
        }
    }
}

/// One end of a placed meter, named as the user clicked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRef {
    /// Stable click-target id (`"gpio8"`, `"led_k"`, `"wiper"`, …).
    pub id: String,
    /// Circuit node (`"gpio8"`, `"0"`, `"vdd"`, …).
    pub node: String,
    /// Short label for the readout list.
    pub label: String,
}

/// A meter the user has placed.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedMeter {
    pub id: u64,
    pub kind: MeterKind,
    pub a: ProbeRef,
    pub b: ProbeRef,
    /// What the worker samples and averages.
    pub meter: Multimeter,
}
