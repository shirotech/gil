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
    /// This method uses a spin loop to wait for available space in the queue.
    /// For a non-blocking alternative, use [`Sender::try_send`].
    pub fn send(&mut self, value: T) {
        let cell = self.ptr.cell_at(self.local_tail);
        let mut backoff = crate::Backoff::with_spin_count(128);
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
