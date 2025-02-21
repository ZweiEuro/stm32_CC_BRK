mod patterns;

pub use patterns::*;

pub struct Settings {
    pub current_patterns: [PeriodPattern<8>; 8],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            current_patterns: [PeriodPattern::new_const(); 8],
        }
    }
}

impl Settings {
    pub fn add_pattern(&mut self, pattern: PeriodPattern<8>) {
        for i in 0..self.current_patterns.len() {
            if self.current_patterns[i].size == 0 {
                self.current_patterns[i] = pattern;
                return;
            }
        }
        // if we get here, we have no space for the pattern
        panic!("No space for pattern");
    }
}
