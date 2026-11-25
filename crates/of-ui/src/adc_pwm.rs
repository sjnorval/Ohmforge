//! Potentiometer dimmer and LED fade simulations for the desktop demos.

mod schematic;

use std::sync::OnceLock;

use of_soc_esp32c3::Esp32c3;

use crate::rom;

pub(super) use schematic::show as show_schematic;

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/partitions.bin");
const DIMMER: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/pot_dimmer.bin");
const FADE: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/led_fade.bin");

pub(super) const GPIO_LED: u8 = 8;
pub(super) const GPIO_POT: u8 = 2;
pub(super) const RESISTOR_REF: &str = "R2";
pub(super) const LED_REF: &str = "D1";
pub(super) const POT_REF: &str = "RV1";

const DIMMER_BANNER: &str = "ohmforge esp-hal pot dimmer\n";
const FADE_BANNER: &str = "ohmforge esp-hal led fade\n";

static DIMMER_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();
static FADE_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();

/// Which committed esp-hal app to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PwmKind {
    Dimmer,
    Fade,
}

impl PwmKind {
    fn banner(self) -> &'static str {
        match self {
            Self::Dimmer => DIMMER_BANNER,
            Self::Fade => FADE_BANNER,
        }
    }

    fn app(self) -> &'static [u8] {
        match self {
            Self::Dimmer => DIMMER,
            Self::Fade => FADE,
        }
    }

    fn cache(self) -> &'static OnceLock<Result<Esp32c3, String>> {
        match self {
            Self::Dimmer => &DIMMER_SOC,
            Self::Fade => &FADE_SOC,
        }
    }
}

/// Cached boot result, if the ROM has already been tried for this app.
#[allow(dead_code)]
pub(super) fn cached(kind: PwmKind) -> Option<&'static Result<Esp32c3, String>> {
    kind.cache().get()
}

/// Boot the app through the fetched ROM (cached after the first success or error).
pub(super) fn booted(kind: PwmKind) -> Result<Esp32c3, String> {
    kind.cache()
        .get_or_init(|| rom::boot_app(kind.app(), PARTITIONS, kind.banner()))
        .clone()
}
