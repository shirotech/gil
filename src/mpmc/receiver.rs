use crate::{atomic::Ordering, mpmc::queue::QueuePtr};

/// The consumer end of the MPMC queue.
///
/// This struct is `Clone` and `Send`. It can be shared across threads by cloning it.
#[derive(Clone)]
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
    /// This method uses a spin loop to wait for available data in the queue.
    /// For a non-blocking alternative, use [`Receiver::try_recv`].
    pub fn recv(&mut self) -> T {
        let head = self.ptr.head().fetch_add(1, Ordering::Relaxed);
        let next = head.wrapping_add(1);
        self.local_head = next;

        let cell = self.ptr.at(head);
        let mut backoff = crate::Backoff::with_spin_count(128);
        while cell.epoch().load(Ordering::Acquire) != next {
            backoff.backoff();
        }

        let ret = unsafe { cell.get() };
        cell.epoch()
            .store(head.wrapping_add(self.ptr.capacity), Ordering::Release);

        ret
    }

    /// Attempts to receive a value from the queue without blocking.
    ///
    /// # Returns
    ///
    /// * `Some(value)` if a value is available.
    /// * `None` if the queue is empty.
    pub fn try_recv(&mut self) -> Option<T> {
        use core::cmp::Ordering as Cmp;

        let mut backoff = crate::Backoff::with_spin_count(16);

        loop {
            let cell = self.ptr.at(self.local_head);
            let epoch = cell.epoch().load(Ordering::Acquire);
            let next_epoch = self.local_head.wrapping_add(1);

            match epoch.cmp(&next_epoch) {
                Cmp::Less => return None,
                Cmp::Equal => {
                    match self.ptr.head().compare_exchange_weak(
                        self.local_head,
                        next_epoch,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => {
                            let ret = unsafe { cell.get() };
                            cell.epoch().store(
                                self.local_head.wrapping_add(self.ptr.capacity),
                                Ordering::Release,
                            );
                            self.local_head = next_epoch;
                            return Some(ret);
                        }
                        Err(cur_head) => self.local_head = cur_head,
                    }
                }
                Cmp::Greater => self.local_head = self.ptr.head().load(Ordering::Relaxed),
            }

            backoff.backoff();
        }
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
