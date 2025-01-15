#![no_main]
#![no_std]

use stm32f0xx_hal::{self as hal, pac::TIM1};
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
use cortex_m::{interrupt::Mutex, peripheral::Peripherals as c_m_Peripherals};

// A type definition for the GPIO pin to be used for our LED
type LEDPIN = gpioa::PA4<Output<PushPull>>;

// Make LED pin globally available
static GLED: Mutex<RefCell<Option<LEDPIN>>> = Mutex::new(RefCell::new(None));

// Make timer interrupt registers globally available
static GINT: Mutex<RefCell<Option<Timer<TIM3>>>> = Mutex::new(RefCell::new(None));

// static ADV_TIMER: Mutex<RefCell<Option<Timer<TIM1>>>> = Mutex::new(RefCell::new(None));

// Define an interupt handler, i.e. function to call when interrupt occurs. Here if our external
// interrupt trips when the timer timed out
#[interrupt]
fn TIM3() {
    static mut LED: Option<LEDPIN> = None;
    static mut INT: Option<Timer<TIM3>> = None;

    let led = LED.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // Move LED pin here, leaving a None in its place
            GLED.borrow(cs).replace(None).unwrap()
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
    defmt::info!("BRK UP interrupt");
}

#[interrupt]
fn TIM1_CC() {
    defmt::info!("TIM1_CC interrupt");
}

#[entry]
fn main() -> ! {
    if let (Some(mut p), Some(cp)) = (Peripherals::take(), c_m_Peripherals::take()) {
        cortex_m::interrupt::free(move |cs| {
            let mut rcc = p
                .RCC
                .configure()
                .sysclk(8.mhz())
                .pclk(4.mhz())
                .freeze(&mut p.FLASH);

            let gpioa = p.GPIOA.split(&mut rcc);

            // (Re-)configure PA5 as output
            // Move the pin into our global storage
            let led = gpioa.pa4.into_push_pull_output(cs);
            *GLED.borrow(cs).borrow_mut() = Some(led);

            // Set up a timer expiring after 1s
            // Generate an interrupt when the timer expires
            // Move the timer into our global storage
            let mut timer = Timer::tim3(p.TIM3, Hertz(1), &mut rcc);
            timer.listen(Event::TimeOut);
            *GINT.borrow(cs).borrow_mut() = Some(timer);

            // Set PA9 as a capture pin
            let _ = gpioa.pa9.into_pull_down_input(cs);

            // advanced timer for input capturing
            let tim1 = p.TIM1;
            // Set counting mode to edge aligned = count from 0 to 16bit max
            defmt::assert!(tim1.cr1.read().cen().is_disabled()); // must be disabled (is anyways but just to be sure)
            tim1.cr1.modify(|_, w| w.dir().up());
            tim1.cr1.modify(|_, w| w.cms().edge_aligned());

            let target_frequ = 1000.mhz();

            let timer_f = rcc.clocks.sysclk().0;
            let pclk_ticks_per_timer_period = timer_f / target_frequ.0;
            defmt::info!("ticks per time period: {}", pclk_ticks_per_timer_period);
            let psc: u16 = (pclk_ticks_per_timer_period - 1).try_into().unwrap();

            // Set prescaler to 0
            tim1.psc.write(|w| w.psc().bits(1));
            tim1.egr.write(|w| w.ug().update());

            // set input TI enable
            // ccmr1 = capture/compare mode register 1
            tim1.ccmr1_input( /* We want channel 2 */)
                .write(|w| w.cc2s().ti1());
            tim1.ccmr1_input().write(|w| w.ic2f().bits(0)); // no filter

            // set to both edges
            // CC1P register
            tim1.ccer.write(|w| w.cc2p().set_bit().cc2np().set_bit());

            // remove presacle
            unsafe {
                // not sure why this is marked unsafe?
                tim1.ccmr1_input().write(|w| w.ic2psc().bits(0));
            }

            // enable channel2
            tim1.ccer.write(|w| w.cc2e().set_bit());

            // enable input interrupt
            tim1.dier.write(|w| w.cc2ie().set_bit());

            // enable timer
            tim1.cr1.modify(|_, w| w.cen().set_bit());

            // Enable TIM7 IRQ, set prio 1 and clear any pending IRQs
            let mut nvic = cp.NVIC;

            unsafe {
                nvic.set_priority(Interrupt::TIM3, 1);
                nvic.set_priority(Interrupt::TIM1_BRK_UP_TRG_COM, 2);
                nvic.set_priority(Interrupt::TIM1_CC, 3);
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
