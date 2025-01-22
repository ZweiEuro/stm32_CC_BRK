#![no_main]
#![no_std]

#[cfg(not(any(feature = "clock_8_mhz")))]
compile_error!("A clock frequency must be specified");

mod input;
mod patterns;

use {defmt_rtt as _, panic_probe as _};

use input::{enable_input_capture, process, setup_timer};
use patterns::Settings;
use static_cell::StaticCell;
use stm32f0xx_hal::{
    gpio::{self, *},
    pac::{interrupt, Interrupt, Peripherals, TIM1, TIM3},
    prelude::*,
    time::Hertz,
    timers::*,
};

use core::{cell::RefCell, convert::TryInto, panic};
use cortex_m::{
    asm::{self, wfe},
    interrupt::Mutex,
    peripheral::Peripherals as c_m_Peripherals,
};
use cortex_m_rt::entry;

// A type definition for the GPIO pin to be used for our LED
type OnboardLedPin = gpioa::PA4<Output<PushPull>>;
type ControlLed = gpioa::PA3<Output<PushPull>>;

type CcLed = gpioa::PA10<Output<PushPull>>;

// Make LED pin globally available
static ONBOARD_LED: Mutex<RefCell<Option<OnboardLedPin>>> = Mutex::new(RefCell::new(None));
static CONTROL_LED: Mutex<RefCell<Option<ControlLed>>> = Mutex::new(RefCell::new(None));

// Make timer interrupt registers globally available
static GINT: Mutex<RefCell<Option<Timer<TIM3>>>> = Mutex::new(RefCell::new(None));

// Define an interupt handler, i.e. function to call when interrupt occurs. Here if our external
// interrupt trips when the timer timed out
#[interrupt]
fn TIM3() {
    static mut LED: Option<OnboardLedPin> = None;
    static mut INT: Option<Timer<TIM3>> = None;
    static mut CONTROL: Option<ControlLed> = None;

    let led = LED.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            ONBOARD_LED.borrow(cs).replace(None).unwrap()
        })
    });

    let int = INT.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            GINT.borrow(cs).replace(None).unwrap()
        })
    });

    let control = CONTROL.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            CONTROL_LED.borrow(cs).replace(None).unwrap()
        })
    });

    led.toggle().ok();
    if led.is_set_high().unwrap() {
        // The onboard LED is active-low
        // we are using a control LED instead in order to make it less confusing

        control.set_low().ok();
    } else {
        control.set_high().ok();
    }

    int.wait().ok();
}

#[entry]
fn main() -> ! {
    if let Some(cp) = c_m_Peripherals::take() {
        let mut nvic = cp.NVIC;

        unsafe {
            nvic.set_priority(Interrupt::TIM3, 0b1000);
            nvic.set_priority(Interrupt::TIM1_BRK_UP_TRG_COM, 0b0001);
            nvic.set_priority(Interrupt::TIM1_CC, 0b0010);
        }
    } else {
        panic!("Failed to take core peripherals");
    }

    if let Some(mut p) = Peripherals::take() {
        cortex_m::interrupt::free(move |cs| {
            p.RCC.apb2enr.modify(|_, w| w.tim1en().set_bit());

            let mut rcc = p
                .RCC
                .configure()
                .sysclk(8.mhz())
                .pclk(4.mhz())
                .freeze(&mut p.FLASH);

            let gpioa = p.GPIOA.split(&mut rcc);

            {
                // (Re-)configure PA4 as output
                // Move the pin into our global storage
                let led = gpioa.pa4.into_push_pull_output(cs);
                *ONBOARD_LED.borrow(cs).borrow_mut() = Some(led);

                // This is just a control LED. With a logic analyser it makes it really easy to see when the interrupt is triggered
                let control_led = gpioa.pa3.into_push_pull_output(cs);
                *CONTROL_LED.borrow(cs).borrow_mut() = Some(control_led);
            }

            {
                // Set up a timer expiring after 1s
                // Generate an interrupt when the timer expires
                // This is used to test input capture by toggling PA4
                let mut timer = Timer::tim3(p.TIM3, Hertz(1), &mut rcc);
                timer.listen(Event::TimeOut);
                *GINT.borrow(cs).borrow_mut() = Some(timer);
            }

            {
                // Set PA9 as a capture pin
                let _ = gpioa.pa9.into_alternate_af2(cs);
                setup_timer(&p.TIM1);
            }
        });
    } else {
        panic!("Failed to take peripherals");
    }

    defmt::info!("Hello, world!");

    static SETTINGS: StaticCell<Settings> = StaticCell::new();
    let settings = SETTINGS.init(Settings::default());

    let sync_bit = patterns::PeriodPattern::new([360, 11160, 0, 0, 0, 0, 0, 0], 0.15);
    let high_bit = patterns::PeriodPattern::new([360, 1080, 360, 1080, 0, 0, 0, 0], 0.15);
    let low_bit = patterns::PeriodPattern::new([360, 1080, 1080, 360, 0, 0, 0, 0], 0.15);

    settings.add_pattern(sync_bit);
    settings.add_pattern(high_bit);
    settings.add_pattern(low_bit);

    // Setup communication between interrupt and main thread

    // wait for a bit
    asm::delay(4_000_000);

    enable_input_capture();

    loop {
        process(settings);
    }
}
