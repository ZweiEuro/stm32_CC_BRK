#![no_main]
#![no_std]

use stm32f0xx_hal::{
    self as hal,
    pac::{rcc, TIM1},
};
use {defmt_rtt as _, panic_probe as _};

use crate::hal::{
    gpio::*,
    pac::{interrupt, Interrupt, Peripherals, TIM3},
    prelude::*,
    time::Hertz,
    timers::*,
};

use cortex_m_rt::entry;

use core::{cell::RefCell, convert::TryInto};
use cortex_m::{
    interrupt::Mutex,
    peripheral::{self, Peripherals as c_m_Peripherals},
};

// A type definition for the GPIO pin to be used for our LED
type OnboardLedPin = gpioa::PA4<Output<PushPull>>;
type CcLed = gpioa::PA10<Output<PushPull>>;

// Make LED pin globally available
static ONBOARD_LED: Mutex<RefCell<Option<OnboardLedPin>>> = Mutex::new(RefCell::new(None));

// Make timer interrupt registers globally available
static GINT: Mutex<RefCell<Option<Timer<TIM3>>>> = Mutex::new(RefCell::new(None));

static ADV_TIMER: Mutex<RefCell<Option<TIM1>>> = Mutex::new(RefCell::new(None));
static CC_LED: Mutex<RefCell<Option<CcLed>>> = Mutex::new(RefCell::new(None));

// Define an interupt handler, i.e. function to call when interrupt occurs. Here if our external
// interrupt trips when the timer timed out
#[interrupt]
fn TIM3() {
    static mut LED: Option<OnboardLedPin> = None;
    static mut INT: Option<Timer<TIM3>> = None;

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

    led.toggle().ok();
    int.wait().ok();
}

#[interrupt]
fn TIM1_BRK_UP_TRG_COM() {
    // clear the interrupt pin
    unsafe {
        let tim1 = &*TIM1::ptr();

        if (tim1.sr.read().uif().bit_is_set()) {
            defmt::info!("tim1 overflowed");
            tim1.sr.write(|w| w.tif().clear_bit());
        } else {
            panic!("interrupt flag not set? Why did this trigger?");
        }
    }
}

#[interrupt]
fn TIM1_CC() {
    static mut LED_CC: Option<CcLed> = None;

    defmt::info!("TIM1_CC interrupt");

    let led_cc = LED_CC.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            CC_LED.borrow(cs).replace(None).unwrap()
        })
    });
    led_cc.toggle().ok();

    // clear the interrupt bit
    unsafe {
        let tim1 = &*TIM1::ptr();
        // tim1.cnt

        defmt::info!("capture value: {}", tim1.ccr1.read().ccr().bits());

        let OC_flag = tim1.sr.read().cc2of().bit_is_set();
        let break_int_flag = tim1.sr.read().bif().bit_is_set();
        let trigger_int_flag = tim1.sr.read().tif().bit_is_set();
        let commutation_int_flag = tim1.sr.read().cc2of().bit_is_set();
        let updatE_interrupt_flag = tim1.sr.read().uif().bit_is_set();

        defmt::info!("OC flag: {}", OC_flag);
        defmt::info!("break interrupt flag: {}", break_int_flag);
        defmt::info!("trigger interrupt flag: {}", trigger_int_flag);
        defmt::info!("commutation interrupt flag: {}", commutation_int_flag);
        defmt::info!("update interrupt flag: {}", updatE_interrupt_flag);

        tim1.sr.write(|w| w.uif().clear_bit());
        tim1.sr.write(|w| w.cc2of().clear_bit());
        tim1.sr.write(|w| w.bif().clear_bit());
        tim1.sr.write(|w| w.tif().clear_bit());
    }
}

#[entry]
fn main() -> ! {
    if let (Some(mut p), Some(cp)) = (Peripherals::take(), c_m_Peripherals::take()) {
        cortex_m::interrupt::free(move |cs| {
            p.RCC.apb2enr.write(|w| w.tim1en().set_bit());

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

                let cc_led = gpioa.pa10.into_push_pull_output(cs);
                *CC_LED.borrow(cs).borrow_mut() = Some(cc_led);
            }

            {
                // Set up a timer expiring after 1s
                // Generate an interrupt when the timer expires
                // Move the timer into our global storage
                let mut timer = Timer::tim3(p.TIM3, Hertz(5), &mut rcc);
                timer.listen(Event::TimeOut);
                *GINT.borrow(cs).borrow_mut() = Some(timer);
            }

            {
                // Set PA9 as a capture pin
                let _ = gpioa.pa9.into_alternate_af2(cs);
            }

            {
                // advanced timer for input capturing
                let tim1 = p.TIM1;

                // Set counting mode to edge aligned = count from 0 to 16bit max
                defmt::assert!(tim1.cr1.read().cen().is_disabled()); // must be disabled (is anyways but just to be sure)

                // set direction and upcounting
                tim1.cr1.write(|w| w.dir().clear_bit()); // 0 = up counting
                tim1.cr1.write(|w| w.cms().edge_aligned()); // count according to direction bit

                // set frequency

                {
                    let f = Hertz(1);

                    let pclk_ticks_per_timer_period = rcc.clocks.sysclk().0 / f.0;

                    let psc: Result<u16, _> = (pclk_ticks_per_timer_period - 1).try_into();

                    if psc.is_ok() {
                        tim1.psc.write(|w| w.psc().bits(psc.unwrap()));
                        defmt::info!("ticks per time period: {}", pclk_ticks_per_timer_period);
                    } else {
                        defmt::warn!("psc value too high");
                        tim1.psc.write(|w| w.psc().bits(100));
                    }

                    // Set prescaler to 0
                    tim1.egr.write(|w| w.ug().set_bit());
                }

                tim1.egr.write(|w| w.ug().set_bit()); // update generation, update thee prescale

                // setup the pin and mode for that pin and channel
                tim1.ccmr1_input().write(|w| w.cc2s().ti2());
                tim1.ccmr1_input().write(|w| w.ic2f().bits(0)); // no filter

                tim1.ccer.write(|w| w.cc2p().set_bit().cc2np().set_bit()); // 00 -> rising edge, 11 -> any edge

                tim1.ccmr1_input().write(|w| unsafe { w.ic2psc().bits(0) }); // no prescaler on the input

                tim1.ccer.write(|w| w.cc2e().set_bit()); // enable capture from this counter into the capture register

                // interrupts

                tim1.dier.write(|w| w.cc2ie().set_bit()); // enable input interrupt

                tim1.dier.write(|w| w.uie().set_bit()); // seems to control the overflow interrupt?

                // tim1.dier.write(|w| w.tie().set_bit());
                // tim1.dier.write(|w| w.bie().set_bit());
                // tim1.dier.write(|w| w.comie().set_bit());

                // enable the counter
                tim1.cr1.write(|w| w.cen().set_bit()); // enable counter

                //grab the timer
                *ADV_TIMER.borrow(cs).borrow_mut() = Some(tim1);
            }

            // Enable TIM7 IRQ, set prio 1 and clear any pending IRQs
            let mut nvic = cp.NVIC;

            unsafe {
                nvic.set_priority(Interrupt::TIM3, 1);
                nvic.set_priority(Interrupt::TIM1_BRK_UP_TRG_COM, 1);
                nvic.set_priority(Interrupt::TIM1_CC, 1);
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
        continue;
    }
}
