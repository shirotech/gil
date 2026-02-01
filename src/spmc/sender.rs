use crate::{atomic::Ordering, spmc::queue::QueuePtr};

/// The producer end of the SPMC queue.
///
/// This struct is `Send` but not `Sync` or `Clone`. It can be moved to another thread, but cannot
/// be shared across threads or cloned.
///
/// # Examples
///
/// ```
/// use core::num::NonZeroUsize;
/// use gil::spmc::channel;
///
/// let (mut tx, mut rx) = channel::<i32>(NonZeroUsize::new(16).unwrap());
/// tx.send(1);
/// tx.send(2);
/// assert_eq!(rx.recv(), 1);
/// assert_eq!(rx.recv(), 2);
/// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spmc::channel;
    ///
    /// let (mut tx, mut rx) = channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// tx.send(42);
    /// assert_eq!(rx.recv(), 42);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spmc::channel;
    ///
    /// let (mut tx, mut rx) = channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// tx.send_with_spin_count(42, 32);
    /// assert_eq!(rx.recv(), 42);
    /// ```
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
    /// Returns `Ok(())` if the value was successfully enqueued, or `Err(value)` if the
    /// queue is full, returning the original value.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spmc::channel;
    ///
    /// let (mut tx, mut rx) = channel::<i32>(NonZeroUsize::new(2).unwrap());
    ///
    /// assert!(tx.try_send(1).is_ok());
    /// assert!(tx.try_send(2).is_ok());
    /// assert_eq!(tx.try_send(3), Err(3));
    ///
    /// assert_eq!(rx.recv(), 1);
    /// ```
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
