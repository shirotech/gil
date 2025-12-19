use crate::{atomic::Ordering, hint, mpmc::queue::QueuePtr, thread};

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

    pub fn recv(&mut self) -> T {
        let head = self.ptr.head().fetch_add(1, Ordering::Relaxed);
        let next = head.wrapping_add(1);
        self.local_head = next;

        let cell = self.ptr.at(head);
        let mut spin_count = 0;
        while cell.epoch().load(Ordering::Acquire) != next {
            if spin_count < 128 {
                hint::spin_loop();
                spin_count += 1;
            } else {
                thread::yield_now();
            }
        }

        let ret = unsafe { cell.get() };
        cell.epoch()
            .store(head.wrapping_add(self.ptr.capacity), Ordering::Release);

        ret
    }

    pub fn try_recv(&mut self) -> Option<T> {
        use std::cmp::Ordering as Cmp;
        let mut spin_count = 0;

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

            if spin_count < 16 {
                crate::hint::spin_loop();
                spin_count += 1;
            } else {
                spin_count = 0;
                crate::thread::yield_now();
            }
        }
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
