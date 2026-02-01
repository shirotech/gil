use core::mem::MaybeUninit;

use crate::atomic::Ordering;

use super::queue::QueuePtr;

/// The producer end of a parking SPSC queue.
///
/// After each write, this sender checks whether the receiver is parked
/// and unparks it if so. The check is a single `Relaxed` atomic load
/// when the receiver is not parked.
pub struct Sender<T> {
    ptr: QueuePtr<T>,
    local_head: usize,
    local_tail: usize,
}

impl<T> Sender<T> {
    pub(crate) fn new(queue_ptr: QueuePtr<T>) -> Self {
        Self {
            ptr: queue_ptr,
            local_head: 0,
            local_tail: 0,
        }
    }

    /// Attempts to send a value into the queue without blocking.
    ///
    /// Returns `Ok(())` if the value was successfully enqueued, or `Err(value)` if the
    /// queue is full, returning the original value.
    pub fn try_send(&mut self, value: T) -> Result<(), T> {
        let new_tail = self.local_tail.wrapping_add(1);

        if new_tail > self.max_tail() {
            self.load_head();
            if new_tail > self.max_tail() {
                return Err(value);
            }
        }

        unsafe { self.ptr.set(self.local_tail, value) };
        self.store_tail(new_tail);
        self.local_tail = new_tail;

        self.ptr.unpark_receiver();

        Ok(())
    }

    /// Sends a value into the queue, blocking if necessary.
    ///
    /// Uses a default spin count of 128.
    pub fn send(&mut self, value: T) {
        self.send_with_spin_count(value, 128);
    }

    /// Sends a value into the queue, blocking if necessary, using a custom spin count.
    pub fn send_with_spin_count(&mut self, value: T, spin_count: u32) {
        let new_tail = self.local_tail.wrapping_add(1);

        let mut backoff = crate::Backoff::with_spin_count(spin_count);
        while new_tail > self.max_tail() {
            backoff.backoff();
            self.load_head();
        }

        unsafe { self.ptr.set(self.local_tail, value) };
        self.store_tail(new_tail);
        self.local_tail = new_tail;

        self.ptr.unpark_receiver();
    }

    /// Returns a mutable slice to the available write buffer in the queue.
    ///
    /// After writing to the buffer, call [`commit`](Sender::commit) to make
    /// the items visible to the receiver.
    pub fn write_buffer(&mut self) -> &mut [MaybeUninit<T>] {
        let mut available = self.ptr.size - self.local_tail.wrapping_sub(self.local_head);

        if available == 0 {
            self.load_head();
            available = self.ptr.size - self.local_tail.wrapping_sub(self.local_head);
        }

        let start = self.local_tail & self.ptr.mask;
        let contiguous = self.ptr.capacity - start;
        let len = available.min(contiguous);

        unsafe {
            let ptr = self.ptr.exact_at(start).cast();
            core::slice::from_raw_parts_mut(ptr.as_ptr(), len)
        }
    }

    /// Commits items written to the buffer obtained via [`write_buffer`](Sender::write_buffer).
    ///
    /// # Safety
    ///
    /// * `len` must be less than or equal to the length of the slice returned by the
    ///   most recent call to [`write_buffer`](Sender::write_buffer).
    /// * All `len` items in the buffer must have been initialized before calling this.
    #[inline(always)]
    pub unsafe fn commit(&mut self, len: usize) {
        #[cfg(debug_assertions)]
        {
            let start = self.local_tail & self.ptr.mask;
            let contiguous = self.ptr.capacity - start;
            let available =
                contiguous.min(self.ptr.size - self.local_tail.wrapping_sub(self.local_head));
            assert!(
                len <= available,
                "advancing ({len}) more than available space ({available})"
            );
        }

        let new_tail = self.local_tail.wrapping_add(len);
        self.store_tail(new_tail);
        self.local_tail = new_tail;

        self.ptr.unpark_receiver();
    }

    #[inline(always)]
    fn max_tail(&self) -> usize {
        self.local_head.wrapping_add(self.ptr.size)
    }

    #[inline(always)]
    fn store_tail(&self, value: usize) {
        self.ptr.tail().store(value, Ordering::Release);
    }

    #[inline(always)]
    fn load_head(&mut self) {
        self.local_head = self.ptr.head().load(Ordering::Acquire);
    }
}

unsafe impl<T: Send> Send for Sender<T> {}
