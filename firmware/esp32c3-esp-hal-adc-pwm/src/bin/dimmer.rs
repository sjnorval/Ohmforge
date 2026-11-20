//! Ohmforge test firmware: an esp-hal potentiometer dimmer.
//!
//! GPIO2 (ADC1 channel 2, 11 dB attenuation) reads a potentiometer wiper
//! with esp-hal's curve-fitting calibration (`AdcCalCurve`, from the chip's
//! eFuse calibration). GPIO8 drives an LED from LEDC channel 0 on timer 0:
//! 1 kHz from the 80 MHz APB clock with 10-bit duty (a fractional divider
//! of 78.125). Every 5 ms the reading sets the duty, 0 V for off to
//! 2500 mV (the top of the calibrated range) and above for full on, and is
//! printed on UART0 as `adc: <mV> duty: <duty>`.

#![no_std]
#![no_main]

use esp_hal::analog::adc::{Adc, AdcCalCurve, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::DriveMode;
use esp_hal::ledc::channel::{self, ChannelHW, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::main;
use esp_hal::peripherals::ADC1;
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// Wiper voltage (mV) that gives full duty.
const FULL_MV: u32 = 2500;
/// Full duty at 10-bit resolution.
const FULL_DUTY: u32 = 1 << 10;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    println!("ohmforge esp-hal pot dimmer");

    let mut adc_config = AdcConfig::new();
    let mut pot = adc_config.enable_pin_with_cal::<_, AdcCalCurve<ADC1>>(peripherals.GPIO2, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);

    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty10Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(1000),
        })
        .unwrap();
    let mut led = ledc.channel(channel::Number::Channel0, peripherals.GPIO8);
    led.configure(channel::config::Config {
        timer: &timer0,
        duty_pct: 0,
        drive_mode: DriveMode::PushPull,
    })
    .unwrap();

    let delay = Delay::new();
    loop {
        let mv = loop {
            if let Ok(v) = adc.read_oneshot(&mut pot) {
                break u32::from(v);
            }
        };
        let duty = mv.min(FULL_MV) * FULL_DUTY / FULL_MV;
        led.set_duty_hw(duty);
        println!("adc: {} duty: {}", mv, duty);
        delay.delay_millis(5);
    }
}
