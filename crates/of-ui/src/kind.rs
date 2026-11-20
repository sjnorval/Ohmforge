//! Which bundled firmware demo the GUI is running.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DemoKind {
    Blink,
    Dimmer,
    Fade,
}

impl DemoKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Blink => "LED blink",
            Self::Dimmer => "Potentiometer dimmer",
            Self::Fade => "LED fade",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Blink => "ESP32-C3 LED blink",
            Self::Dimmer => "ESP32-C3 potentiometer dimmer",
            Self::Fade => "ESP32-C3 LED fade",
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Blink => "Bundled bare-metal firmware toggles GPIO8. No ROM download required.",
            Self::Dimmer => "esp-hal reads the pot on GPIO2 (ADC) and sets a 1 kHz LEDC duty on GPIO8.",
            Self::Fade => "esp-hal fades the LED on GPIO8 up and down in hardware (LEDC, 5 kHz).",
        }
    }

    pub(crate) fn has_pot(self) -> bool {
        matches!(self, Self::Dimmer)
    }

    pub(crate) fn needs_rom(self) -> bool {
        !matches!(self, Self::Blink)
    }
}
