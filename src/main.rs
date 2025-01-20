#![no_main]
#![no_std]

use {defmt_rtt as _, panic_probe as _};

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

static ADV_TIMER: Mutex<RefCell<Option<TIM1>>> = Mutex::new(RefCell::new(None));
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

#[inline]
unsafe fn dump_sr_reg() {
    let tim1 = &*TIM1::ptr();
    // tim1.cnt

    defmt::info!(
        "---- capture value: CR1 {:05} CR2: {:05} Sr-reg {:b}",
        tim1.ccr1.read().ccr().bits(),
        tim1.ccr2.read().ccr().bits(),
        tim1.sr.read().bits()
    );
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

        dump_sr_reg();
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

        defmt::info!("---- TIM1_CC interrupt");
        dump_sr_reg();

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

            {
                /*
                Current problems:
                - Rising edge only, it should do both but it doesn't
                - `update` interrupt is not firing at all?
                - interrupts are cleared without me clearing them
                 */

                // advanced timer for input capturing
                let tim1 = p.TIM1;

                // Set counting mode to edge aligned = count from 0 to 16bit max
                defmt::assert!(tim1.cr1.read().cen().is_disabled()); // must be disabled (is anyways but just to be sure)
                tim1.ccer.modify(|_, w| w.cc2e().clear_bit());

                // 1. Set count direction and alignment

                {
                    // Setup timer, this all seems to work as expected

                    tim1.cr1.modify(|_, w| w.dir().up()); // 0 -> upcounting, 1 -> downcounting
                    tim1.cr1.modify(|_, w| w.cms().edge_aligned()); // edge aligned, count in direction of dir

                    // set timer frequency
                    // Counter frequency is:
                    // CK_CNT = fCK_PSC / (PSC[15:0] + 1)
                    // target_hz = 8Mhz / (PSC + 1)
                    // PSC = (8Mhz / target_hz) - 1
                    let target_hz = Hertz(10000); // 1 ms

                    let psc = (rcc.clocks.sysclk().0 / target_hz.0) - 1;

                    if psc > 0xFFFF {
                        panic!("PSC value too large at {}", psc);
                    }

                    let psc: u16 = psc.try_into().unwrap();

                    tim1.psc.modify(|_, w| w.psc().bits(psc));

                    // manually generate an update to load the new psc
                    tim1.egr.write(|w| w.ug().set_bit());
                }

                // 2. source for counting, which is internal which is default so its fine
                // empty

                // 3. Set input filter
                let filter = 0b0000;
                tim1.ccmr1_input().modify(|_, w| w.ic2f().bits(filter)); // reset value
                tim1.ccmr1_input().modify(|_, w| w.cc2s().ti2());

                /*
                Explicitly setting IC2PSC to 0
                Doing this makes it:
                - Require clearing of IC2F to 0 in TIM1_CC, otherwise it loops forever (not setting ic2psc makes it *NOT* required to clear that flag?)

                Expected: To have the correct PSC behavior, or none at all with 0
                */
                // tim1.ccmr1_input().modify(|_,w| unsafe { w.ic2psc().bits(0) });

                // 4. set input to rising edge
                /*
                Doesn't make a difference for some rason, always rising edge
                Reading CCER also shows that they are 0 even after directly setting them?

                Expected: Correctly triggering TIM1_CC on rising and falling edges
                */
                tim1.ccer.modify(|_, w| w.cc2p().set_bit()); // 00 -> rising edge, 11 -> any edge according to docs
                tim1.ccer.modify(|_, w| w.cc2np().set_bit());

                // 6. Enable capture from counter to the capture register
                tim1.ccer.modify(|_, w| w.cc2e().set_bit());

                // 7. Enable interrupts
                /*
                Strange behavior:
                - Setting UIE=1 and CC2E=1 makes ONLY TIM1_CC work?
                - Setting UIE=1 and CC2E=0 makes TIM1_BRK_UP_TRG_COM work as expected with correct values (on every overflow)
                - URS seems to work as expected but it's hard to tell from the other behavior

                Expected behavior: TIM1_CC should trigger on every rising/falling edge, TIM1_BRK_UP_TRG_COM should trigger on every overflow
                */
                tim1.dier.modify(|_, w| w.uie().set_bit()); // update interrupt
                tim1.cr1.modify(|_, w| w.urs().set_bit()); // only fire update-interrupt on overflow
                tim1.dier.modify(|_, w| w.cc2ie().set_bit()); // capture interrupt

                // 8. Enable the timer
                tim1.cr1.modify(|_, w| w.cen().set_bit()); // enable counter

                //grab the timer
                *ADV_TIMER.borrow(cs).borrow_mut() = Some(tim1);
            }

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
