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

fn dump_flags() {
    unsafe {
        let tim1 = &*TIM1::ptr();
        // tim1.cnt

        defmt::info!("---- capture value: {}", tim1.ccr1.read().ccr().bits());

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

        tim1.sr
            .write(|w: &mut stm32f0xx_hal::pac::tim1::sr::W| w.uif().clear_bit());
        tim1.sr.write(|w| w.cc2of().clear_bit());
        tim1.sr.write(|w| w.bif().clear_bit());
        tim1.sr.write(|w| w.tif().clear_bit());
    }
}

#[interrupt]
fn TIM1_BRK_UP_TRG_COM() {
    defmt::info!("---- TIM1_BRK_UP_TRG_COM interrupt");

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

    defmt::info!("---- TIM1_CC interrupt");

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

        if tim1.sr.read().cc2if().bit_is_set() {
            defmt::info!("---- TIM1_CC interrupt: CC2IF");
        } else if tim1.sr.read().cc1if().bit_is_set() {
            defmt::info!("---- TIM1_CC interrupt: CC1IF");
        } else if tim1.sr.read().uif().bit_is_set() {
            defmt::info!("---- TIM1_CC interrupt: UIF");
        } else {
            defmt::info!("---- TIM1_CC interrupt: unknown");
        }

        tim1.sr.write(|w| w.cc1if().clear_bit());
        tim1.sr.write(|w| w.cc2if().clear_bit());
        tim1.sr.write(|w| w.uif().clear_bit());
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
                let mut timer = Timer::tim3(p.TIM3, Hertz(1), &mut rcc);
                timer.listen(Event::TimeOut);
                *GINT.borrow(cs).borrow_mut() = Some(timer);
            }

            {
                // Set PA9 as a capture pin
                let _ = gpioa.pa9.into_alternate_af2(cs);
            }

            {
                /*
                 * Try to bind TIM1 to the PA9 pin as "input".
                 * TIM1 should count up continuously and on every capture TIM1_CC should fire.
                 * On every "overflow" TIM1_BRK_UP_TRG_COM should fire so we can reset it and handle that case properly
                 *
                 * Optionally: use SMS (Slave mode select) in order to "reset" the counter on each capture
                 *
                 * Steps taken from stm32-hal at https://github.com/David-OConnor/stm32-hal/blob/main/src/timer.rs#L1049
                 * 0. Disable counter and input capture compare
                 * 1. Select active input for TIMx_CCR1, which is the counter capture register. "Capture compare input"
                 *      - In our case we want TIM1 to count CCR1 up
                 *      - disable the capture compare first: CCMR.CC1E = 0  <- done before
                 *      - CCMR.CC1S to 01 -> CC1 channel will be mapped to TI1
                 * 2. Select a proper source
                 *      - Default is internal clock which is fine
                 * 3. Input filter of the COUNTER
                 *      - Since we are counting from internal we divide the internal clock by this value before counting
                 *      - This is "the minimum up-time for a signal needed to be counted as valid"
                 *      - but since we are using the internal clock we can set this to 0
                 *      - BUT in the example i am following its hard coded to 0011 so i am using that
                 * 4. Set the polarity of the input capture:
                 *      - CCER.CC1P = 0, CCER.CC1NP = 0 -> rising edge
                 *      - CCER.CC1P = 1, CCER.CC1NP = 1 -> any edge    <- This is eventually what i want
                 * 5. Set the prescalar for the counter input source (which is the internal clock)
                 *      - PSC = 0 would make it overflow constantly, i wanne slow it down a bit
                 * 6. Enable capturing from the counter into the capturing register
                 * 7. Enable "update" interrupt and "capture compare" interrupt
                 */
                // advanced timer for input capturing
                let tim1 = p.TIM1;

                // Set counting mode to edge aligned = count from 0 to 16bit max
                // 0
                defmt::assert!(tim1.cr1.read().cen().is_disabled()); // must be disabled (is anyways but just to be sure)
                tim1.ccer.write(|w| w.cc1e().clear_bit()); // disable capture compare channel 1
                tim1.ccer.write(|w| w.cc2e().clear_bit()); // disable capture compare channel 2

                // 1. Set count direction and alignment
                tim1.cr1.write(|w| w.dir().up()); // 0 -> upcounting, 1 -> downcounting
                tim1.cr1.write(|w| w.cms().edge_aligned()); // edge aligned, count in direction of dir
                tim1.ccmr1_input().write(|w| w.cc1s().ti1());

                // 2. source for counting, which is internal which is default so its fine

                // 3. Set input filter
                let filter = 0b0011;
                tim1.ccmr1_input().write(|w| w.ic2f().bits(filter));

                // 4. set input to rising edge
                tim1.ccer.write(|w| w.cc2p().clear_bit()); // 00 -> rising edge, 11 -> any edge
                tim1.ccer.write(|w| w.cc2np().clear_bit());

                // 5. Set prescaler
                let psc = 0b010;
                tim1.psc.write(|w| w.psc().bits(psc));

                // 6. Enable capture from counter to the capture register
                tim1.ccer.write(|w| w.cc2e().set_bit());
                tim1.ccer.write(|w| w.cc1e().set_bit());

                // 7. Enable interrupts
                tim1.dier.write(|w| w.uie().set_bit()); // seems to control the overflow interrupt?
                tim1.dier.write(|w| w.cc2ie().set_bit()); // seems to control the capture interrupt?

                // only make the UPDATE interrupt trigger on overflow
                tim1.cr1.write(|w| w.urs().set_bit()); // only update on overflow

                // enable the counter
                tim1.cr1.write(|w| w.cen().set_bit()); // enable counter

                //grab the timer
                *ADV_TIMER.borrow(cs).borrow_mut() = Some(tim1);
            }

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
