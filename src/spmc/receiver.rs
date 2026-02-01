use crate::{atomic::Ordering, spmc::queue::QueuePtr};

/// The consumer end of the SPMC queue.
///
/// Unlike SPSC receivers, this struct implements `Clone`, allowing multiple consumers to receive
/// from the same queue. Each consumer competes for items - an item will be received by exactly
/// one consumer.
///
/// This struct is `Send` but not `Sync`. It can be moved to another thread, but cannot be shared
/// across threads without cloning.
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
    /// This method uses a spin loop to wait for a value to become available.
    /// For a non-blocking alternative, use [`Receiver::try_recv`].
    ///
    /// When multiple receivers exist, each call to `recv` competes with other receivers.
    /// Exactly one receiver will get each item.
    pub fn recv(&mut self) -> T {
        let head = self.ptr.head().fetch_add(1, Ordering::Relaxed);
        let next_head = head.wrapping_add(1);

        let cell = self.ptr.cell_at(head);
        let mut backoff = crate::Backoff::with_spin_count(128);
        while cell.epoch().load(Ordering::Acquire) != next_head {
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
    /// * `Some(value)` if a value was successfully received.
    /// * `None` if the queue is empty or another receiver claimed the item.
    pub fn try_recv(&mut self) -> Option<T> {
        use core::cmp::Ordering as Cmp;

        let mut backoff = crate::Backoff::with_spin_count(16);
        loop {
            let cell = self.ptr.cell_at(self.local_head);
            let epoch = cell.epoch().load(Ordering::Acquire);
            let next_head = self.local_head.wrapping_add(1);

            match epoch.cmp(&next_head) {
                Cmp::Less => return None,
                Cmp::Equal => {
                    match self.ptr.head().compare_exchange_weak(
                        self.local_head,
                        next_head,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => {
                            let ret = unsafe { cell.get() };
                            cell.epoch().store(
                                self.local_head.wrapping_add(self.ptr.capacity),
                                Ordering::Release,
                            );
                            self.local_head = next_head;
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

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr.clone(),
            local_head: self.ptr.head().load(Ordering::Relaxed),
        }
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
