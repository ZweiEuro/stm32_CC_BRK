use {defmt_rtt as _, panic_probe as _};

use stm32f0xx_hal::{
    pac::{Interrupt, TIM1},
    time::Hertz,
};

use core::{convert::TryInto, panic};

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

pub fn disable_input_capture(tim1: &TIM1) {
    // Disable capture interrupt
    unsafe {
        let tim1 = &*TIM1::ptr();

        // Enable capture interrupt
        tim1.ccer.modify(|_, w| w.cc2e().clear_bit());
    }
}
