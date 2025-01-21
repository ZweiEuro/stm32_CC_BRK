#[derive(Debug, Copy, Clone)]
pub struct PeriodPattern {
    pub periods: [u16; 8],
    pub size: u8,
}

impl Default for PeriodPattern {
    fn default() -> Self {
        Self::new_const()
    }
}

impl PeriodPattern {
    pub fn new(periods: [u16; 8]) -> Self {
        // go through the pattern and the first 0 denotes the end of the pattern
        let size = periods.iter().position(|&x| x == 0).unwrap_or(8) as u8;
        Self {
            periods,
            size: size,
        }
    }

    pub const fn new_const() -> Self {
        Self {
            periods: [0; 8],
            size: 0,
        }
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
