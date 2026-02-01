use crate::{atomic::Ordering, spmc::queue::QueuePtr};

/// The producer end of the SPMC queue.
///
/// This struct is `Send` but not `Sync` or `Clone`. It can be moved to another thread, but cannot
/// be shared across threads or cloned.
pub struct Sender<T> {
    ptr: QueuePtr<T>,
    local_tail: usize,
}

impl<T> Sender<T> {
    pub(crate) fn new(queue_ptr: QueuePtr<T>) -> Self {
        Self {
            ptr: queue_ptr,
            local_tail: 0,
        }
    }

    /// Sends a value into the queue, blocking if necessary.
    ///
    /// This method uses a spin loop with a default spin count of 128 to wait
    /// for available space in the queue. For control over the spin count, use
    /// [`Sender::send_with_spin_count`]. For a non-blocking alternative, use
    /// [`Sender::try_send`].
    pub fn send(&mut self, value: T) {
        self.send_with_spin_count(value, 128);
    }

    /// Sends a value into the queue, blocking if necessary, using a custom spin count.
    ///
    /// The `spin_count` controls how many times the backoff spins before yielding
    /// the thread. A higher value keeps the thread spinning longer, which can reduce
    /// latency when the queue is expected to drain quickly, at the cost of higher CPU
    /// usage. A lower value yields sooner, reducing CPU usage but potentially
    /// increasing latency.
    ///
    /// For a non-blocking alternative, use [`Sender::try_send`].
    pub fn send_with_spin_count(&mut self, value: T, spin_count: u32) {
        let cell = self.ptr.cell_at(self.local_tail);
        let mut backoff = crate::Backoff::with_spin_count(spin_count);
        while cell.epoch().load(Ordering::Acquire) != self.local_tail {
            backoff.backoff();
        }

        let next = self.local_tail.wrapping_add(1);
        cell.set(value);
        cell.epoch().store(next, Ordering::Release);
        self.local_tail = next;
    }

    /// Attempts to send a value into the queue without blocking.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the value was successfully sent.
    /// * `Err(value)` if the queue is full, returning the original value.
    pub fn try_send(&mut self, value: T) -> Result<(), T> {
        let cell = self.ptr.cell_at(self.local_tail);
        if cell.epoch().load(Ordering::Acquire) != self.local_tail {
            return Err(value);
        }

        let next = self.local_tail.wrapping_add(1);
        cell.set(value);
        cell.epoch().store(next, Ordering::Release);
        self.local_tail = next;

        Ok(())
    }
}

unsafe impl<T: Send> Send for Sender<T> {}
