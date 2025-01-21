use crate::patterns::Settings;

use {defmt_rtt as _, panic_probe as _};

use cortex_m::interrupt::Mutex;
use ringbuffer::RingBuffer;
use stm32f0xx_hal::{
    pac::{interrupt, Interrupt, TIM1},
    time::Hertz,
};

use core::{
    cell::{Cell, RefCell},
    convert::TryInto,
    panic,
};

// Timer we use input capturing on
pub fn setup_timer(tim1: &TIM1) {
    // must be disabled for config
    defmt::assert!(tim1.cr1.read().cen().is_disabled());
    // Disable capture interrupt
    tim1.ccer.modify(|_, w| w.cc2e().clear_bit());

    {
        // Setup timer, this all seems to work as expected
        // 1. Set count direction and alignment

        tim1.cr1.modify(|_, w| w.dir().up().cms().edge_aligned()); // edge aligned -> count in direction of dir

        // set timer frequency
        // Counter frequency is:
        // CK_CNT = fCK_PSC / (PSC[15:0] + 1)
        // target_hz = 8Mhz / (PSC + 1)
        // PSC = (8Mhz / target_hz) - 1

        // how fast the timer counts
        // 1 / target_timer_frequ_hz = time value (in seconds) of 1 tick on the timer
        #[cfg(feature = "res_micro")]
        let target_timer_frequ_hz = Hertz(1_000_000);

        #[cfg(feature = "clock_8_mhz")]
        let clock_freq_hz = Hertz(8_000_000);

        let psc = (clock_freq_hz.0 / target_timer_frequ_hz.0) - 1;

        if psc > 0xFFFF {
            panic!("PSC value too large at {}", psc);
        }

        let psc: u16 = psc.try_into().unwrap(); // this will crash should the value not fit into psc

        tim1.psc.modify(|_, w| w.psc().bits(psc));

        // manually generate an update to load the new psc
        tim1.egr.write(|w| w.ug().set_bit());
    }

    // 3. Set input filter
    let filter: u8 = 0b1111; // sample with 8 samples, normal frequency
    tim1.ccmr1_input().modify(|_, w| w.ic2f().bits(filter)); // reset value
    tim1.ccmr1_input().modify(|_, w| w.cc2s().ti2());

    // 4. set input to rising and falling edge
    // 00 -> rising edge, 11 -> any edge according to docs
    tim1.ccer
        .modify(|_, w| w.cc2p().set_bit().cc2np().set_bit());

    // enable reset mode, reset the counter each capture, giving us the time between captures
    tim1.smcr.modify(|_, w| w.sms().reset_mode());
    tim1.smcr.modify(|_, w| w.ts().ti2fp2()); // trigger on input 2

    // 6. Enable capture from counter to the capture register
    // Do NOT enable it by default
    //tim1.ccer.modify(|_, w| w.cc2e().set_bit());

    // 7. Enable interrupts
    tim1.dier.modify(|_, w| w.uie().set_bit()); // update interrupt
    tim1.cr1.modify(|_, w| w.urs().set_bit()); // only fire update-interrupt on overflow
    tim1.dier.modify(|_, w| w.cc2ie().set_bit()); // capture interrupt

    // 8. Enable the timer
    tim1.cr1.modify(|_, w| w.cen().set_bit()); // enable counter

    // Enable interrupts in masking registers
    unsafe {
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM3);
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM1_BRK_UP_TRG_COM);
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM1_CC);
    }
    cortex_m::peripheral::NVIC::unpend(Interrupt::TIM3);
    cortex_m::peripheral::NVIC::unpend(Interrupt::TIM1_BRK_UP_TRG_COM);
    cortex_m::peripheral::NVIC::unpend(Interrupt::TIM1_CC);
}

pub fn enable_input_capture() {
    unsafe {
        let tim1 = &*TIM1::ptr();

        // Enable capture interrupt
        tim1.ccer.modify(|_, w| w.cc2e().set_bit());
    }
}

pub fn disable_input_capture() {
    // Disable capture interrupt
    unsafe {
        let tim1 = &*TIM1::ptr();

        // Enable capture interrupt
        tim1.ccer.modify(|_, w| w.cc2e().clear_bit());
    }
}

static mut TIM1_OVERFLOWED: bool = false;

#[interrupt]
fn TIM1_BRK_UP_TRG_COM() {
    // clear the interrupt pin
    unsafe {
        let tim1 = &*TIM1::ptr();

        if tim1.sr.read().uif().bit_is_clear() {
            panic!("interrupt flag not set? Why did this trigger?");
        }

        tim1.sr.modify(|_, w| w.uif().clear_bit());
        TIM1_OVERFLOWED = true;
    }
}

const BUFFER_SIZE: usize = 8;

struct BufferState {
    buffer: [u16; BUFFER_SIZE],
    next_index: u8, // needed although internally known for ring buffer to reconstruct the window
    dirty: bool,
}

impl BufferState {
    pub fn new() -> Self {
        Self {
            buffer: [0; BUFFER_SIZE],
            next_index: 0,
            dirty: false,
        }
    }

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

    pub fn get_window(&self) -> [u16; BUFFER_SIZE] {
        let mut window = [0; BUFFER_SIZE];

        // we want the element that was last written to
        let window_start = self.next_index as usize;

        // copy the next BUFFER_SIZE elements into the window
        // the modulo operation is needed to wrap around the buffer

        for window_index in 0..BUFFER_SIZE {
            let value_index = (window_start + window_index) % BUFFER_SIZE;
            let val = self.buffer[value_index];
            if val == 0 {
                return window;
            } else {
                window[window_index] = val;
            }
        }

        window
    }
}

static GLOBAL_DATA: Mutex<RefCell<Option<BufferState>>> =
    Mutex::new(RefCell::new(Some(BufferState::new_const())));

#[interrupt]
fn TIM1_CC() {
    unsafe {
        // clear the interrupt bit
        let tim1 = &*TIM1::ptr();
        let period = tim1.ccr2.read().bits() as u16;
        tim1.sr.modify(|_, w| w.cc2if().clear_bit());

        if !TIM1_OVERFLOWED && period > 200 {
            // filter out any noise
            // or large gaps

            cortex_m::interrupt::free(|cs| {
                let mut buf_ref = GLOBAL_DATA.borrow(cs).borrow_mut();

                let buf_ref = buf_ref.as_mut().unwrap();

                buf_ref.push(period);
            });
        }

        // can be done in any case, checking the if would take more cyles
        TIM1_OVERFLOWED = false;
    }
}

pub fn process(settings: &Settings) {
    let mut current_window = [0; BUFFER_SIZE];

    cortex_m::interrupt::free(|cs| {
        let mut buf_ref = GLOBAL_DATA.borrow(cs).borrow_mut();

        if !buf_ref.is_none() {
            let buf_ref = buf_ref.as_mut().unwrap();

            if buf_ref.dirty {
                current_window = buf_ref.get_window();
                buf_ref.dirty = false;
            } else {
                return;
            }
        }
    });

    if current_window[0] == 0 {
        return;
    }

    defmt::info!("Current window: {:?}", current_window);

    // check against all available patterns and if there is a hit print it out
    let tolerance: f32 = 0.3;

    for pattern in settings.current_patterns {
        if pattern.size == 0 {
            continue;
        }

        for signal_index in 0..BUFFER_SIZE {
            let target_val = f32::from(pattern.periods[signal_index]);
            let window_period = f32::from(current_window[signal_index]);

            if target_val == 0.0 {
                defmt::info!("Pattern hit! Signal end");

                defmt::info!(
                    "Pattern hit! Pattern {} window {}",
                    pattern.periods,
                    current_window
                );

                loop {
                    cortex_m::asm::nop();
                }

                break;
            }

            if window_period == 0.0 {
                // miss for sure
                break;
            }

            if !(target_val * (1.0 - tolerance) < window_period
                && window_period < target_val * (1.0 + tolerance))
            {
                // the signal value is out of tolerance
                break;
            }
        }
    }
}
