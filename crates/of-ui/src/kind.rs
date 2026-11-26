//! Which bundled demo the GUI is running: the ESP32-C3 firmware demos and
//! the 555 timer circuits (no microcontroller).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DemoKind {
    Blink,
    Dimmer,
    Fade,
    ButtonPoll,
    ButtonIrq,
    /// NE555 astable LED flasher (`ne555-astable`).
    Ne555Astable,
    /// NE555 monostable one-shot on a push button (`ne555-monostable`).
    Ne555Monostable,
    /// NE555 two-diode PWM dimmer on a pot (`ne555-pwm-dimmer`).
    Ne555Pwm,
    /// An NE555 clock counted by ESP32-C3 firmware (`esp32c3-555-clock`).
    Clock555,
}

/// The demo picker's entries, in menu order.
pub(crate) const PICKER: [DemoKind; 9] = [
    DemoKind::Blink,
    DemoKind::Dimmer,
    DemoKind::Fade,
    DemoKind::ButtonPoll,
    DemoKind::ButtonIrq,
    DemoKind::Ne555Astable,
    DemoKind::Ne555Monostable,
    DemoKind::Ne555Pwm,
    DemoKind::Clock555,
];

impl DemoKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Blink => "LED blink",
            Self::Dimmer => "Potentiometer dimmer",
            Self::Fade => "LED fade",
            Self::ButtonPoll => "Push button (poll)",
            Self::ButtonIrq => "Push button (irq)",
            Self::Ne555Astable => "555 astable flasher",
            Self::Ne555Monostable => "555 one-shot (button)",
            Self::Ne555Pwm => "555 PWM dimmer",
            Self::Clock555 => "555 clock counter",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Blink => "ESP32-C3 LED blink",
            Self::Dimmer => "ESP32-C3 potentiometer dimmer",
            Self::Fade => "ESP32-C3 LED fade",
            Self::ButtonPoll => "ESP32-C3 push button (poll)",
            Self::ButtonIrq => "ESP32-C3 push button (irq)",
            Self::Ne555Astable => "NE555 astable LED flasher",
            Self::Ne555Monostable => "NE555 monostable one-shot",
            Self::Ne555Pwm => "NE555 PWM LED dimmer",
            Self::Clock555 => "ESP32-C3 555 clock counter",
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Blink => "Bundled bare-metal firmware toggles GPIO8. No ROM download required.",
            Self::Dimmer => "esp-hal reads the pot on GPIO2 (ADC) and sets a 1 kHz LEDC duty on GPIO8.",
            Self::Fade => "esp-hal fades the LED on GPIO8 up and down in hardware (LEDC, 5 kHz).",
            Self::ButtonPoll => {
                "esp-hal polls GPIO9 (internal pull-up, SW1 to ground) and copies it to the LED on GPIO8."
            }
            Self::ButtonIrq => "esp-hal wakes from wfi on a falling edge at GPIO9 and toggles the LED on GPIO8.",
            Self::Ne555Astable => {
                "R1 1 kOhm, R2 10 kOhm and C1 10 uF set about 6.7 Hz: C1 swings between VCC/3 and 2VCC/3."
            }
            Self::Ne555Monostable => {
                "Press SW1: the output and LED stay on for 1.1 RC (about 110 ms), however long you hold it."
            }
            Self::Ne555Pwm => {
                "The pot splits the timing resistance between the charge and discharge diodes: duty 18 % to 91 % at about 1 kHz."
            }
            Self::Clock555 => {
                "A 5 V NE555 clocks GPIO4 through a 10k/33k divider; bare-metal firmware counts edges over 100 ms and prints the frequency on UART0. No ROM download required."
            }
        }
    }

    pub(crate) fn has_pot(self) -> bool {
        matches!(self, Self::Dimmer | Self::Ne555Pwm)
    }

    pub(crate) fn has_button(self) -> bool {
        matches!(self, Self::ButtonPoll | Self::ButtonIrq | Self::Ne555Monostable)
    }

    pub(crate) fn needs_rom(self) -> bool {
        self.has_mcu() && !matches!(self, Self::Blink | Self::Clock555)
    }

    /// The desktop's scope and meters come from the file's `[[instrument]]`
    /// tables, and the run starts on the file's supply: the 555 demos and
    /// the clock counter (the other ESP32-C3 demos keep their own).
    pub(crate) fn file_bench(self) -> bool {
        !self.has_mcu() || matches!(self, Self::Clock555)
    }

    /// An ESP32-C3 (`U1`) runs firmware in this demo; the 555 demos are
    /// analog only.
    pub(crate) fn has_mcu(self) -> bool {
        !matches!(self, Self::Ne555Astable | Self::Ne555Monostable | Self::Ne555Pwm)
    }
}
