pub struct Backoff {
    spin_count: u32,
    current: u32,
}

impl Backoff {
    #[inline(always)]
    pub fn with_spin_count(spin_count: u32) -> Self {
        Self {
            spin_count,
            current: 0,
        }
    }

    #[inline(always)]
    pub fn set_spin_count(&mut self, spin_count: u32) {
        self.spin_count = spin_count;
    }

    #[inline(always)]
    pub fn backoff(&mut self) {
        if self.current < self.spin_count {
            crate::hint::spin_loop();
            self.current += 1;
        } else {
            self.current = 0;
            crate::thread::yield_now();
        }
    }

    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
    }
}
