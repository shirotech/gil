use crate::{atomic::Ordering, mpsc::queue::QueuePtr};

/// The consumer end of the queue.
///
/// This struct is `Send` but not `Sync`. It can be moved to another thread, but cannot be shared
/// across threads.
pub struct Receiver<T> {
    ptr: QueuePtr<T>,
    local_head: usize,
}

impl<T> Receiver<T> {
    pub(crate) fn new(queue_ptr: QueuePtr<T>) -> Self {
        Self {
            ptr: queue_ptr,
            local_head: 0,
        }
    }

    /// Receives a value from the queue, blocking if necessary.
    ///
    /// This method uses a spin loop with a default spin count of 16 to wait
    /// for available data in the queue. For control over the spin count, use
    /// [`Receiver::recv_with_spin_count`]. For a non-blocking alternative, use
    /// [`Receiver::try_recv`].
    pub fn recv(&mut self) -> T {
        self.recv_with_spin_count(16)
    }

    /// Receives a value from the queue, blocking if necessary, using a custom spin count.
    ///
    /// The `spin_count` controls how many times the backoff spins before yielding
    /// the thread. A higher value keeps the thread spinning longer, which can reduce
    /// latency when the queue is expected to fill quickly, at the cost of higher CPU
    /// usage. A lower value yields sooner, reducing CPU usage but potentially
    /// increasing latency.
    ///
    /// For a non-blocking alternative, use [`Receiver::try_recv`].
    pub fn recv_with_spin_count(&mut self, spin_count: u32) -> T {
        let next_head = self.local_head.wrapping_add(1);

        let cell = self.ptr.cell_at(self.local_head);
        let mut backoff = crate::Backoff::with_spin_count(spin_count);
        while cell.epoch().load(Ordering::Acquire) < next_head {
            backoff.backoff();
        }

        let ret = unsafe { cell.get() };
        cell.epoch().store(
            self.local_head.wrapping_add(self.ptr.capacity),
            Ordering::Release,
        );

        self.local_head = next_head;

        ret
    }

    /// Attempts to receive a value from the queue without blocking.
    ///
    /// # Returns
    ///
    /// * `Some(value)` if a value is available.
    /// * `None` if the queue is empty.
    pub fn try_recv(&mut self) -> Option<T> {
        let next_head = self.local_head.wrapping_add(1);

        let cell = self.ptr.cell_at(self.local_head);
        if cell.epoch().load(Ordering::Acquire) < next_head {
            return None;
        }

        let ret = unsafe { cell.get() };
        cell.epoch().store(
            self.local_head.wrapping_add(self.ptr.capacity),
            Ordering::Release,
        );

        self.local_head = next_head;

        Some(ret)
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
