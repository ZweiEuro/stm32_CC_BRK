mod signalbuffer;
use crate::patterns::Settings;

use {defmt_rtt as _, panic_probe as _};

use cortex_m::interrupt::Mutex;

use signalbuffer::SignalWindow;
use stm32f0xx_hal::{
    pac::{interrupt, Interrupt, TIM3},
    time::Hertz,
};

use core::{
    cell::{Ref, RefCell},
    convert::TryInto,
    panic,
};

static INPUT_CAPTURE: Mutex<RefCell<Option<InputCapture>>> = Mutex::new(RefCell::new(None));

pub struct InputCapture {
    tim3: TIM3,
    overflow_counter: u16,
    signal_window: SignalWindow<8>,
}

// Timer we use input capturing on
// We wanne use TIM3_CH1 -> bound to PA6 on alternative function 1
impl InputCapture {
    pub fn init(tim3: TIM3, pclk: Hertz) {
        unsafe {
            // really simple gate
            static mut FIRST: bool = true;

            if FIRST {
                FIRST = false;
            } else {
                panic!("InputCapture singleton already initialized");
            }
        }

        // must be disabled for config
        defmt::assert!(tim3.cr1.read().cen().is_disabled());

        tim3.ccer.modify(|_, w| w.cc1e().clear_bit());

        // configure as input capture mode
        tim3.ccmr1_input().modify(|_, w| w.cc1s().ti1());

        {
            // Setup timer, this all seems to work as expected
            // 1. Set count direction and alignment

            tim3.cr1.modify(|_, w| w.dir().up().cms().edge_aligned()); // edge aligned -> count in direction of dir

            // set timer frequency
            // Counter frequency is:
            // CK_CNT = fCK_PSC / (PSC[15:0] + 1)
            // target_hz = 8Mhz / (PSC + 1)
            // PSC = (8Mhz / target_hz) - 1

            // how fast the timer counts
            // 1 / target_timer_frequ_hz = time value (in seconds) of 1 tick on the timer
            #[cfg(feature = "res_micro")]
            let target_timer_frequ_hz = Hertz(1_000_000);

            let psc = (pclk.0 / target_timer_frequ_hz.0) - 1;

            if psc > 0xFFFF {
                panic!("PSC value too large at {}", psc);
            }

            let psc: u16 = psc.try_into().unwrap(); // this will crash should the value not fit into psc

            tim3.psc.modify(|_, w| w.psc().bits(psc));

            // manually generate an update to load the new psc
            tim3.egr.write(|w| w.ug().set_bit());
        }

        // 3. Set input filter
        let filter: u8 = 0b0000; // sample with 8 samples, normal frequency
        tim3.ccmr1_input().modify(|_, w| w.ic1f().bits(filter));

        // 4. set input to rising and falling edge
        // 00 -> rising edge, 11 -> any edge
        tim3.ccer
            .modify(|_, w| w.cc1p().set_bit().cc1np().set_bit());

        // enable reset mode, reset the counter each capture, giving us the time between captures
        tim3.smcr.modify(|_, w| w.sms().reset_mode());
        tim3.smcr.modify(|_, w| w.ts().ti1fp1());

        // 7. Enable interrupts
        tim3.ccer.modify(|_, w| w.cc1e().set_bit());
        tim3.dier.modify(|_, w| w.cc1ie().set_bit()); // capture interrupt

        tim3.cr1.modify(|_, w| w.urs().set_bit()); // only fire update-interrupt on overflow
        tim3.dier.modify(|_, w| w.uie().set_bit()); // enable updated interrupts
        tim3.dier.modify(|_, w| w.tie().set_bit()); // enable trigger interrupts

        tim3.cr1.modify(|_, w| w.cen().set_bit()); // enable counter

        // Enable interrupts in masking registers
        unsafe {
            cortex_m::peripheral::NVIC::unmask(Interrupt::TIM3);
        }
        cortex_m::peripheral::NVIC::unpend(Interrupt::TIM3);

        cortex_m::interrupt::free(|cs| {
            defmt::info!("Setting up input capture singleton");

            let prev = INPUT_CAPTURE.borrow(cs).borrow_mut().replace(Self {
                tim3: tim3,
                overflow_counter: 0,
                signal_window: SignalWindow::new_const(),
            });

            defmt::assert!(prev.is_none());

            defmt::assert!(INPUT_CAPTURE.borrow(cs).borrow_mut().is_some());
        });

        defmt::info!("Timer setup done");
        // check that it exists
        InputCapture::input_capture_singleton(|input_capture_ref| {});
    }

    pub fn input_capture_singleton<F, R>(f: F) -> R
    where
        F: FnOnce(&mut InputCapture) -> R,
    {
        cortex_m::interrupt::free(|cs| {
            let mut input_capture_ref = INPUT_CAPTURE.borrow(cs).borrow_mut();

            if input_capture_ref.is_none() {
                panic!("InputCapture singleton not initialized");
            }

            let input_capture_ref = input_capture_ref.as_mut().unwrap();

            f(input_capture_ref)
        })
    }

    pub fn enable_input_capture() {
        InputCapture::input_capture_singleton(|input_capture_ref| {
            input_capture_ref
                .tim3
                .ccer
                .modify(|_, w| w.cc1e().set_bit()); // enable counter
        });
    }

    pub fn disable_input_capture() {
        InputCapture::input_capture_singleton(|input_capture_ref| {
            input_capture_ref
                .tim3
                .ccer
                .modify(|_, w| w.cc1e().clear_bit()); // enable counter
        });
    }

    /**
     * Handle the interrupt flags and if a capture has happened return the period and reset the overflow counter
     */
    pub fn handle_interrupt() -> Option<u32> {
        return InputCapture::input_capture_singleton(|input_capture_ref| {
            let sr = input_capture_ref.tim3.sr.read();

            if sr.uif().bit_is_set() {
                input_capture_ref.overflow_counter += 1;
            }

            let ret: Option<u32>;

            if sr.cc1if().bit_is_set() {
                let period = input_capture_ref.tim3.ccr1.read().bits() as u32
                    + ((input_capture_ref.overflow_counter as u32) << 16);
                input_capture_ref.overflow_counter = 0;
                ret = Some(period);
            } else {
                ret = None;
            }

            input_capture_ref.tim3.sr.reset();

            if let Some(value) = ret {
                if value > 20 {
                    defmt::info!("Capture value: {}", value);
                    input_capture_ref.signal_window.push(value);
                }
            }

            return ret;
        });
    }
}

#[interrupt]
fn TIM3() {
    InputCapture::handle_interrupt();
}

pub fn process(settings: &Settings) {
    InputCapture::input_capture_singleton(|input_capture_ref| {
        if input_capture_ref.signal_window.dirty {
            let (current_window, window_start_index) = input_capture_ref.signal_window.get_window();
            input_capture_ref.signal_window.dirty = false;

            for (i, pattern) in settings.current_patterns.iter().enumerate() {
                if pattern.match_window(&current_window) {
                    if i == 0 {
                        defmt::info!("\n SYNC bit");
                    }
                    defmt::info!(
                        "Pattern hit! Pattern {} window {}",
                        pattern.periods,
                        current_window
                    );

                    input_capture_ref
                        .signal_window
                        .clear_region(window_start_index, pattern.size as usize);
                }
            }
        } else {
            return;
        }
    });
}
