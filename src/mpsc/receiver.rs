use crate::{atomic::Ordering, hint, mpsc::queue::QueuePtr, thread};

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

    pub fn recv(&mut self) -> T {
        let next_head = self.local_head.wrapping_add(1);

        let cell = self.ptr.at(self.local_head);
        let mut spin_count = 0;
        while cell.epoch().load(Ordering::Acquire) < next_head {
            if spin_count < 128 {
                hint::spin_loop();
                spin_count += 1;
            } else {
                thread::yield_now();
            }
        }

        let ret = unsafe { cell.get() };
        cell.epoch()
            .store(self.local_head + self.ptr.capacity, Ordering::Release);

        self.local_head = next_head;

        ret
    }

    pub fn try_recv(&mut self) -> Option<T> {
        let next_head = self.local_head.wrapping_add(1);

        let cell = self.ptr.at(self.local_head);
        if cell.epoch().load(Ordering::Acquire) < next_head {
            return None;
        }

        let ret = unsafe { cell.get() };
        cell.epoch()
            .store(self.local_head + self.ptr.capacity, Ordering::Release);

        self.local_head = next_head;

        Some(ret)
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
