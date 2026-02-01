use crate::atomic::Ordering;

use super::queue::QueuePtr;

/// The consumer end of a parking SPSC queue.
///
/// When the queue is empty, the receiver spins briefly, then yields,
/// then parks the thread via [`std::thread::park`]. The sender will
/// unpark it when new data arrives.
pub struct Receiver<T> {
    ptr: QueuePtr<T>,
    local_tail: usize,
    local_head: usize,
}

impl<T> Receiver<T> {
    pub(crate) fn new(queue_ptr: QueuePtr<T>) -> Self {
        Self {
            ptr: queue_ptr,
            local_tail: 0,
            local_head: 0,
        }
    }

    /// Attempts to receive a value from the queue without blocking.
    ///
    /// Returns `Some(value)` if a value is available, or `None` if the queue is empty.
    pub fn try_recv(&mut self) -> Option<T> {
        if self.local_head == self.local_tail {
            self.load_tail();
            if self.local_head == self.local_tail {
                return None;
            }
        }

        let ret = unsafe { self.ptr.get(self.local_head) };
        let new_head = self.local_head.wrapping_add(1);
        self.store_head(new_head);
        self.local_head = new_head;

        Some(ret)
    }

    /// Receives a value from the queue, parking the thread when idle.
    ///
    /// Uses a three-phase backoff with default spin count 128 and yield
    /// count 10. After both phases are exhausted the thread is parked
    /// until the sender writes new data.
    pub fn recv(&mut self) -> T {
        self.recv_with_parking(128, 10)
    }

    /// Receives a value, with custom spin and yield counts before parking.
    ///
    /// * `spin_count` — number of [`core::hint::spin_loop`] iterations.
    /// * `yield_count` — number of [`std::thread::yield_now`] calls after
    ///   spinning before the thread parks.
    pub fn recv_with_parking(&mut self, spin_count: u32, yield_count: u32) -> T {
        // Fast path: data already locally visible
        if self.local_head != self.local_tail {
            return self.consume();
        }

        self.load_tail();
        if self.local_head != self.local_tail {
            return self.consume();
        }

        // Slow path: spin → yield → park
        let mut backoff = crate::ParkingBackoff::new(spin_count, yield_count);
        loop {
            if !backoff.backoff() {
                // Prepare to park
                self.ptr.store_receiver_thread();
                self.ptr.set_receiver_parked(true);

                // Re-check after setting the flag to prevent lost wakeup:
                // if the sender wrote between our last load_tail and setting
                // the flag, we'll see it here.
                self.load_tail();
                if self.local_head != self.local_tail {
                    self.ptr.set_receiver_parked(false);
                    return self.consume();
                }

                std::thread::park();
                self.ptr.set_receiver_parked(false);
                backoff.reset();
            }

            self.load_tail();
            if self.local_head != self.local_tail {
                return self.consume();
            }
        }
    }

    /// Returns a slice of the available read buffer in the queue.
    ///
    /// After reading from the buffer, call [`advance`](Receiver::advance) to
    /// mark the items as consumed.
    pub fn read_buffer(&mut self) -> &[T] {
        let mut available = self.local_tail.wrapping_sub(self.local_head);

        if available == 0 {
            self.load_tail();
            available = self.local_tail.wrapping_sub(self.local_head);
        }

        let start = self.local_head & self.ptr.mask;
        let contiguous = self.ptr.capacity - start;
        let len = available.min(contiguous);

        unsafe {
            let ptr = self.ptr.exact_at(start);
            core::slice::from_raw_parts(ptr.as_ptr(), len)
        }
    }

    /// Advances the consumer head by `len` items.
    ///
    /// # Safety
    ///
    /// * `len` must be less than or equal to the length of the slice returned by the
    ///   most recent call to [`read_buffer`](Receiver::read_buffer).
    #[inline(always)]
    pub unsafe fn advance(&mut self, len: usize) {
        #[cfg(debug_assertions)]
        {
            let start = self.local_head & self.ptr.mask;
            let contiguous = self.ptr.capacity - start;
            let available = contiguous.min(self.local_tail.wrapping_sub(self.local_head));
            assert!(
                len <= available,
                "advancing ({len}) more than available space ({available})"
            );
        }

        let new_head = self.local_head.wrapping_add(len);
        self.store_head(new_head);
        self.local_head = new_head;
    }

    // ── private helpers ─────────────────────────────────────────────

    /// Consumes one item at the current head. Caller must ensure head != tail.
    #[inline(always)]
    fn consume(&mut self) -> T {
        // SAFETY: head != tail means queue is not empty and head has a valid
        //         initialised value
        let ret = unsafe { self.ptr.get(self.local_head) };
        let new_head = self.local_head.wrapping_add(1);
        self.store_head(new_head);
        self.local_head = new_head;
        ret
    }

    #[inline(always)]
    fn store_head(&self, value: usize) {
        self.ptr.head().store(value, Ordering::Release);
    }

    #[inline(always)]
    fn load_tail(&mut self) {
        self.local_tail = self.ptr.tail().load(Ordering::Acquire);
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
