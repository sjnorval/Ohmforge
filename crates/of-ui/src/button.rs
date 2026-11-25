//! Push-button demos: esp-hal polls or interrupts on GPIO9, LED on GPIO8.

mod schematic;

use std::sync::OnceLock;

use of_soc_esp32c3::Esp32c3;

use crate::rom;

pub(super) use schematic::show as show_schematic;

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/partitions.bin");
const POLL: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_poll.bin");
const IRQ: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_irq.bin");

pub(super) const GPIO_LED: u8 = 8;
pub(super) const GPIO_BTN: u8 = 9;
pub(super) const RESISTOR_REF: &str = "R2";
pub(super) const LED_REF: &str = "D1";
pub(super) const SWITCH_REF: &str = "SW1";

const POLL_BANNER: &str = "ohmforge esp-hal button (polling)\n";
const IRQ_BANNER: &str = "ohmforge esp-hal button (interrupt)\n";

static POLL_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();
static IRQ_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();

/// Which committed esp-hal button app to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Poll,
    Irq,
}

impl ButtonKind {
    fn banner(self) -> &'static str {
        match self {
            Self::Poll => POLL_BANNER,
            Self::Irq => IRQ_BANNER,
        }
    }

    fn app(self) -> &'static [u8] {
        match self {
            Self::Poll => POLL,
            Self::Irq => IRQ,
        }
    }

    fn cache(self) -> &'static OnceLock<Result<Esp32c3, String>> {
        match self {
            Self::Poll => &POLL_SOC,
            Self::Irq => &IRQ_SOC,
        }
    }
}

/// Cached boot result, if the ROM has already been tried for this app.
#[allow(dead_code)]
pub(super) fn cached(kind: ButtonKind) -> Option<&'static Result<Esp32c3, String>> {
    kind.cache().get()
}

/// Boot the app through the fetched ROM (cached after the first success or error).
pub(super) fn booted(kind: ButtonKind) -> Result<Esp32c3, String> {
    kind.cache()
        .get_or_init(|| rom::boot_app(kind.app(), PARTITIONS, kind.banner()))
        .clone()
}
