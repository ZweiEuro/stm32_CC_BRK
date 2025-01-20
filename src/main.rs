#![no_main]
#![no_std]

#[cfg(not(any(feature = "clock_8_mhz")))]
compile_error!("A clock frequency must be specified");

mod input;

use {defmt_rtt as _, panic_probe as _};

use input::setup_timer;
use stm32f0xx_hal::{
    pac::TIM1,
    {
        gpio::*,
        pac::{interrupt, Interrupt, Peripherals, TIM3},
        prelude::*,
        time::Hertz,
        timers::*,
    },
};

use core::{cell::RefCell, convert::TryInto, panic};
use cortex_m::{asm, interrupt::Mutex, peripheral::Peripherals as c_m_Peripherals};
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

static CC_LED: Mutex<RefCell<Option<CcLed>>> = Mutex::new(RefCell::new(None));

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

#[interrupt]
fn TIM1_BRK_UP_TRG_COM() {
    defmt::info!("---- TIM1_BRK_UP_TRG_COM interrupt");
    // clear the interrupt pin
    unsafe {
        let tim1 = &*TIM1::ptr();

        if tim1.sr.read().uif().bit_is_set() {
            defmt::info!("tim1 overflowed");
            //tim1.sr.modify(|_,w| w.uif().clear_bit());
        } else {
            panic!("interrupt flag not set? Why did this trigger?");
        }

        tim1.sr.modify(|_, w| w.uif().clear_bit());
    }
}

#[interrupt]
fn TIM1_CC() {
    static mut LED_CC: Option<CcLed> = None;

    let led_cc = LED_CC.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            CC_LED.borrow(cs).replace(None).unwrap()
        })
    });
    led_cc.toggle().ok();

    unsafe {
        // clear the interrupt bit
        let tim1 = &*TIM1::ptr();

        defmt::info!("---- TIM1_CC interrupt {:05}", tim1.ccr2.read().bits());

        // for some reason not needed ? (if UIE=1 and CC2E=1)
        tim1.sr.modify(|_, w| w.cc2if().clear_bit());
    }
}

#[entry]
fn main() -> ! {
    if let (Some(mut p), Some(cp)) = (Peripherals::take(), c_m_Peripherals::take()) {
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

                // PA4 is the onbaord LED, but PA4 is active low. This is the same just flipped so on a logic analyser it's easier to see
                let not_pa4 = gpioa.pa10.into_push_pull_output(cs);
                *CC_LED.borrow(cs).borrow_mut() = Some(not_pa4);
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
            }

            setup_timer(&p.TIM1);

            // Set prio and clear flags
            let mut nvic = cp.NVIC;

            unsafe {
                nvic.set_priority(Interrupt::TIM3, 0b1000);
                nvic.set_priority(Interrupt::TIM1_BRK_UP_TRG_COM, 0b0001);
                nvic.set_priority(Interrupt::TIM1_CC, 0b0010);

                cortex_m::peripheral::NVIC::unmask(Interrupt::TIM3);
                cortex_m::peripheral::NVIC::unmask(Interrupt::TIM1_BRK_UP_TRG_COM);
                cortex_m::peripheral::NVIC::unmask(Interrupt::TIM1_CC);
            }
            cortex_m::peripheral::NVIC::unpend(Interrupt::TIM3);
            cortex_m::peripheral::NVIC::unpend(Interrupt::TIM1_BRK_UP_TRG_COM);
            cortex_m::peripheral::NVIC::unpend(Interrupt::TIM1_CC);
        });
    }

    loop {
        asm::wfe();
        continue;
    }
}
