use crate::patterns::Settings;

use {defmt_rtt as _, panic_probe as _};

use cortex_m::interrupt::Mutex;

use stm32f0xx_hal::{
    pac::{interrupt, Interrupt, TIM3},
    time::Hertz,
};

use core::{cell::RefCell, convert::TryInto, panic};

// Timer we use input capturing on
// We wanne use TIM3_CH1 -> bound to PA6 on alternative function 1
pub fn setup_timer(tim3: &TIM3, pclk: Hertz) {
    // must be disabled for config
    defmt::assert!(tim3.cr1.read().cen().is_disabled());

    // Disable capture interrupt
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

        defmt::info!("PSC: {}", psc);

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

    defmt::info!("Timer setup done");
}

pub fn enable_input_capture() {
    unsafe {
        let tim3 = &*TIM3::ptr();

        // Enable capture interrupt and counter
        tim3.ccer.modify(|_, w| w.cc1e().set_bit()); // enable counter
    }
}

pub fn disable_input_capture() {
    unsafe {
        let tim3 = &*TIM3::ptr();
        tim3.ccer.modify(|_, w| w.cc1e().clear_bit()); // enable counter
    }
}

const BUFFER_SIZE: usize = 8;

struct BufferState {
    buffer: [u16; BUFFER_SIZE],
    next_index: u8, // needed although internally known for ring buffer to reconstruct the window
    dirty: bool,
}

impl BufferState {
    pub const fn new_const() -> Self {
        Self {
            buffer: [0; BUFFER_SIZE],
            next_index: 0,
            dirty: false,
        }
    }

    pub fn push(&mut self, value: u16) {
        self.buffer[self.next_index as usize] = value;
        self.next_index = (self.next_index + 1) % (BUFFER_SIZE as u8);
        self.dirty = true;
    }

    /**
     * Return the window that is currently relevant and the start index of the window inside the buffer
     */
    pub fn get_window(&self) -> ([u16; BUFFER_SIZE], usize) {
        let mut window = [0; BUFFER_SIZE];

        // we want the element that was last written to
        let window_start = self.next_index as usize;

        // copy the next BUFFER_SIZE elements into the window
        // the modulo operation is needed to wrap around the buffer

        for window_index in 0..BUFFER_SIZE {
            let value_index = (window_start + window_index) % BUFFER_SIZE;
            let val = self.buffer[value_index];
            if val == 0 {
                return (window, window_start);
            } else {
                window[window_index] = val;
            }
        }

        (window, window_start)
    }

    /**
     * Clear from `start` `count` number of elements. Sets it all to 0
     * - This circles back around should `start + end` be larger than the buffer
     */
    pub fn clear_region(&mut self, start: usize, count: usize) {
        for index in start..start + count {
            self.buffer[index % BUFFER_SIZE] = 0;
        }

        defmt::info!("buffer: {}", self.buffer);
    }
}

static GLOBAL_DATA: Mutex<RefCell<Option<BufferState>>> =
    Mutex::new(RefCell::new(Some(BufferState::new_const())));

static mut TIM_OVERFLOWED: bool = false;

static mut TIM_OVERFLOW_COUNTER: u16 = 0;

#[interrupt]
fn TIM3() {
    unsafe {
        // clear the interrupt bit
        let tim3 = &*TIM3::ptr();

        if tim3.sr.read().uif().bit_is_set() {
            TIM_OVERFLOW_COUNTER += 1;
        }

        if tim3.sr.read().cc1if().bit_is_set() {
            defmt::info!(
                "period: {:?}",
                tim3.ccr1.read().bits() as u32 + 65536 * TIM_OVERFLOW_COUNTER as u32
            );
            TIM_OVERFLOW_COUNTER = 0;
        }

        tim3.sr.reset();
        return;

        if tim3.sr.read().cc1of().is_overcapture() {
            tim3.sr.modify(|_, w| w.cc1of().clear_bit());
            defmt::info!("TIM3 OverCapture");
        }

        if tim3.sr.read().uif().bit_is_set() {
            tim3.sr.modify(|_, w| w.uif().clear_bit());
            TIM_OVERFLOWED = true;
        }

        if tim3.sr.read().cc1if().bit_is_set() {
            tim3.sr.modify(|_, w| w.cc1if().clear_bit());

            let period = tim3.ccr1.read().bits() as u16;

            defmt::info!("Period: {}", period);
            if !TIM_OVERFLOWED && period > 200 && period < 20000 {
                // filter out any noise
                // or large gaps

                cortex_m::interrupt::free(|cs| {
                    let mut buf_ref = GLOBAL_DATA.borrow(cs).borrow_mut();

                    let buf_ref = buf_ref.as_mut().unwrap();

                    buf_ref.push(period);
                });
            }
            // can be done in any case, checking the if would take more cyles
            TIM_OVERFLOWED = false;
        }
    }
}

pub fn process(settings: &Settings) {
    let mut current_window = [0; BUFFER_SIZE];

    cortex_m::interrupt::free(|cs| {
        let mut buf_ref = GLOBAL_DATA.borrow(cs).borrow_mut();

        if !buf_ref.is_none() {
            let buf_ref = buf_ref.as_mut().unwrap();

            if buf_ref.dirty {
                let window_start_index;
                (current_window, window_start_index) = buf_ref.get_window();
                buf_ref.dirty = false;

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

                        buf_ref.clear_region(window_start_index, pattern.size as usize);
                    }
                }
            } else {
                return;
            }
        }
    });

    if current_window[0] == 0 {
        return;
    }

    // defmt::info!("Current window: {:?}", current_window);

    // check against all available patterns and if there is a hit print it out
}
