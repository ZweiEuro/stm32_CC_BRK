pub struct SignalWindow<const BUFFER_SIZE: usize> {
    buffer: [u32; BUFFER_SIZE],
    next_index: u8, // needed although internally known for ring buffer to reconstruct the window
    pub dirty: bool,
}

impl<const BUFFER_SIZE: usize> SignalWindow<BUFFER_SIZE> {
    pub const fn new_const() -> Self {
        Self {
            buffer: [0; BUFFER_SIZE],
            next_index: 0,
            dirty: false,
        }
    }

    pub fn push(&mut self, value: u32) {
        self.buffer[self.next_index as usize] = value;
        self.next_index = (self.next_index + 1) % (BUFFER_SIZE as u8);
        self.dirty = true;
    }

    /**
     * Return the window that is currently relevant and the start index of the window inside the buffer
     */
    pub fn get_window(&self) -> ([u32; BUFFER_SIZE], usize) {
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
