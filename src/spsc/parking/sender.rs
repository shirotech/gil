use core::mem::MaybeUninit;

use crate::atomic::Ordering;

use super::queue::QueuePtr;

/// The producer end of a parking SPSC queue.
///
/// # Examples
///
/// ```
/// use core::num::NonZeroUsize;
/// use gil::spsc::parking;
///
/// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(16).unwrap());
/// tx.send(1);
/// tx.send(2);
/// assert_eq!(rx.recv(), 1);
/// assert_eq!(rx.recv(), 2);
/// ```
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

    /// Attempts to send a value without blocking.
    ///
    /// Returns `Ok(())` if the value was enqueued, or `Err(value)` if the
    /// queue is full.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(2).unwrap());
    ///
    /// assert!(tx.try_send(1).is_ok());
    /// assert!(tx.try_send(2).is_ok());
    /// assert!(tx.try_send(3).is_err()); // full
    /// ```
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

        self.ptr.try_unpark_receiver();

        Ok(())
    }

    /// Sends a value, parking the thread if the queue is full.
    ///
    /// Uses default spin count 128 and yield count 10.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// tx.send(42);
    /// assert_eq!(rx.recv(), 42);
    /// ```
    pub fn send(&mut self, value: T) {
        self.send_with_parking(value, 128, 10);
    }

    /// Sends a value with custom spin and yield counts before parking.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// // 256 spins, 20 yields before parking
    /// tx.send_with_parking(42, 256, 20);
    /// assert_eq!(rx.recv(), 42);
    /// ```
    pub fn send_with_parking(&mut self, value: T, spin_count: u32, yield_count: u32) {
        let new_tail = self.local_tail.wrapping_add(1);

        if new_tail > self.max_tail() {
            self.load_head();
            if new_tail > self.max_tail() {
                let mut backoff = crate::ParkingBackoff::new(spin_count, yield_count);
                loop {
                    backoff.backoff(|| {
                        self.ptr.set_sender_parked();

                        // SeqCst load: pairs with the SeqCst store in set_sender_parked
                        // to form the Dekker pattern (store parked → load head).
                        self.local_head = self.ptr.head().load(Ordering::SeqCst);
                        if new_tail <= self.max_tail() {
                            self.ptr.clear_sender_parked();
                            return;
                        }

                        self.ptr.wait_as_sender();
                    });

                    self.load_head();
                    if new_tail <= self.max_tail() {
                        break;
                    }
                }
            }
        }

        unsafe { self.ptr.set(self.local_tail, value) };
        self.store_tail(new_tail);
        self.local_tail = new_tail;

        self.ptr.try_unpark_receiver();
    }

    /// Returns a mutable slice of the available write buffer.
    ///
    /// After writing to the buffer, call [`commit`](Sender::commit) to make
    /// the items visible to the receiver.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<u64>(NonZeroUsize::new(16).unwrap());
    ///
    /// let buf = tx.write_buffer();
    /// let n = buf.len().min(3);
    /// for i in 0..n {
    ///     buf[i].write(i as u64);
    /// }
    /// unsafe { tx.commit(n) };
    ///
    /// for i in 0..n {
    ///     assert_eq!(rx.try_recv(), Some(i as u64));
    /// }
    /// ```
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

    /// Commits items written via [`write_buffer`](Sender::write_buffer).
    ///
    /// # Safety
    ///
    /// * `len` must be `<=` the length of the most recent [`write_buffer`](Sender::write_buffer) slice.
    /// * All `len` items must have been initialized.
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

        self.ptr.try_unpark_receiver();
    }

    #[inline(always)]
    fn max_tail(&self) -> usize {
        self.local_head.wrapping_add(self.ptr.size)
    }

    #[inline(always)]
    fn store_tail(&self, value: usize) {
        self.ptr.tail().store(value, Ordering::SeqCst);
    }

    #[inline(always)]
    fn load_head(&mut self) {
        self.local_head = self.ptr.head().load(Ordering::Acquire);
    }
}

unsafe impl<T: Send> Send for Sender<T> {}
