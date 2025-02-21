#[derive(Debug, Copy, Clone)]
pub struct PeriodPattern<const PERIOD_SIZE: usize> {
    pub periods: [u16; PERIOD_SIZE],
    pub size: u8,
    pub tolerance: f64,
}

impl<const PERIOD_SIZE: usize> Default for PeriodPattern<PERIOD_SIZE> {
    fn default() -> Self {
        Self::new_const()
    }
}

impl<const PERIOD_SIZE: usize> PeriodPattern<PERIOD_SIZE> {
    pub fn new(periods: [u16; PERIOD_SIZE], tolerance: f64) -> Self {
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
    pub fn match_window(&self, signal_pattern: &[u32; PERIOD_SIZE]) -> bool {
        if self.size == 0 {
            return false;
        }

        for signal_index in 0..PERIOD_SIZE {
            let target_val = f64::from(self.periods[signal_index]);
            let signal_period = f64::from(signal_pattern[signal_index]);

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
pub struct PeriodPatternIter<'a, const PERIOD_SIZE: usize> {
    pattern: &'a PeriodPattern<PERIOD_SIZE>,
    index: u8,
}

impl<'a, const PERIOD_SIZE: usize> Iterator for PeriodPatternIter<'a, PERIOD_SIZE> {
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

impl<'a, const PERIOD_SIZE: usize> IntoIterator for &'a PeriodPattern<PERIOD_SIZE> {
    type Item = u16;
    type IntoIter = PeriodPatternIter<'a, PERIOD_SIZE>;

    fn into_iter(self) -> Self::IntoIter {
        PeriodPatternIter {
            pattern: self,
            index: 0,
        }
    }
}
