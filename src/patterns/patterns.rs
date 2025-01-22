const PERIOD_SIZE: usize = 8;

#[derive(Debug, Copy, Clone)]
pub struct PeriodPattern {
    pub periods: [u16; PERIOD_SIZE],
    pub size: u8,
    pub tolerance: f32,
}

impl Default for PeriodPattern {
    fn default() -> Self {
        Self::new_const()
    }
}

impl PeriodPattern {
    pub fn new(periods: [u16; PERIOD_SIZE], tolerance: f32) -> Self {
        // go through the pattern and the first 0 denotes the end of the pattern
        let size = periods.iter().position(|&x| x == 0).unwrap() as u8;
        Self {
            periods,
            size: size,
            tolerance: tolerance,
        }
    }

    pub const fn new_const() -> Self {
        Self {
            periods: [0; PERIOD_SIZE],
            size: 0,
            tolerance: 0.0,
        }
    }

    #[inline]
    pub fn match_window(&self, signal_pattern: &[u16; PERIOD_SIZE]) -> bool {
        if self.size == 0 {
            return false;
        }

        for signal_index in 0..PERIOD_SIZE {
            let target_val = f32::from(self.periods[signal_index]);
            let signal_period = f32::from(signal_pattern[signal_index]);

            if target_val == 0.0 {
                // we are 'done'
                return true;
            }

            if signal_period == 0.0 {
                // miss for sure
                return false;
            }

            if !(target_val * (1.0 - self.tolerance) < signal_period
                && signal_period < target_val * (1.0 + self.tolerance))
            {
                // the signal value is out of tolerance
                return false;
            } else {
                #[cfg(feature = "debug_recv")]
                defmt::info!(
                    "Pattern hit! Signal {:06} < {:06} < {:06}",
                    target_val * (1.0 - self.tolerance),
                    signal_period,
                    target_val * (1.0 + self.tolerance)
                );
            }
        }

        true
    }
}

// create a read only interator
pub struct PeriodPatternIter<'a> {
    pattern: &'a PeriodPattern,
    index: u8,
}

impl<'a> Iterator for PeriodPatternIter<'a> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.pattern.size {
            let period = self.pattern.periods[self.index as usize];
            self.index += 1;
            Some(period)
        } else {
            None
        }
    }
}

impl<'a> IntoIterator for &'a PeriodPattern {
    type Item = u16;
    type IntoIter = PeriodPatternIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        PeriodPatternIter {
            pattern: self,
            index: 0,
        }
    }
}
