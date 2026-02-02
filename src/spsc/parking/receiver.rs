use crate::atomic::Ordering;

use super::queue::QueuePtr;

/// The consumer end of a parking SPSC queue.
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
pub struct Receiver<T> {
    ptr: QueuePtr<T>,
    local_tail: usize,
    local_head: usize,
}

impl<T> Receiver<T> {
    pub(crate) fn new(queue_ptr: QueuePtr<T>) -> Self {
        Self {
            ptr: queue_ptr,
            local_tail: 0,
            local_head: 0,
        }
    }

    /// Attempts to receive a value without blocking.
    ///
    /// Returns `Some(value)` if data is available, or `None` if the queue
    /// is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// assert_eq!(rx.try_recv(), None);
    /// tx.send(42);
    /// assert_eq!(rx.try_recv(), Some(42));
    /// ```
    pub fn try_recv(&mut self) -> Option<T> {
        if self.local_head == self.local_tail {
            self.load_tail();
            if self.local_head == self.local_tail {
                return None;
            }
        }

        let ret = unsafe { self.ptr.get(self.local_head) };
        let new_head = self.local_head.wrapping_add(1);
        self.store_head(new_head);
        self.local_head = new_head;

        self.ptr.unpark();

        Some(ret)
    }

    /// Receives a value, parking the thread if the queue is empty.
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
    pub fn recv(&mut self) -> T {
        self.recv_with_parking(128, 10)
    }

    /// Receives a value with custom spin and yield counts before parking.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<i32>(NonZeroUsize::new(16).unwrap());
    /// tx.send(42);
    /// // 256 spins, 20 yields before parking
    /// assert_eq!(rx.recv_with_parking(256, 20), 42);
    /// ```
    pub fn recv_with_parking(&mut self, spin_count: u32, yield_count: u32) -> T {
        // Fast path: data already locally visible
        if self.local_head != self.local_tail {
            return self.consume();
        }

        self.load_tail();
        if self.local_head != self.local_tail {
            return self.consume();
        }

        let mut backoff = crate::ParkingBackoff::new(spin_count, yield_count);
        loop {
            backoff.backoff(|| {
                self.ptr.store_thread();
                self.ptr.set_parked(true);

                // catch lost unparks
                self.load_tail();
                if self.local_head != self.local_tail {
                    self.ptr.set_parked(false);
                    return;
                }

                crate::thread::park();
                self.ptr.set_parked(false);
            });

            self.load_tail();
            if self.local_head != self.local_tail {
                return self.consume();
            }
        }
    }

    /// Returns a slice of the available read buffer.
    ///
    /// After reading, call [`advance`](Receiver::advance) to mark items as
    /// consumed.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::spsc::parking;
    ///
    /// let (mut tx, mut rx) = parking::channel::<u64>(NonZeroUsize::new(16).unwrap());
    /// tx.send(10);
    /// tx.send(20);
    ///
    /// let buf = rx.read_buffer();
    /// assert_eq!(buf[0], 10);
    /// assert_eq!(buf[1], 20);
    /// unsafe { rx.advance(2) };
    /// ```
    pub fn read_buffer(&mut self) -> &[T] {
        let mut available = self.local_tail.wrapping_sub(self.local_head);

        if available == 0 {
            self.load_tail();
            available = self.local_tail.wrapping_sub(self.local_head);
        }

        let start = self.local_head & self.ptr.mask;
        let contiguous = self.ptr.capacity - start;
        let len = available.min(contiguous);

        unsafe {
            let ptr = self.ptr.exact_at(start);
            core::slice::from_raw_parts(ptr.as_ptr(), len)
        }
    }

    /// Advances the consumer head by `len` items.
    ///
    /// # Safety
    ///
    /// * `len` must be `<=` the length of the most recent [`read_buffer`](Receiver::read_buffer) slice.
    #[inline(always)]
    pub unsafe fn advance(&mut self, len: usize) {
        #[cfg(debug_assertions)]
        {
            let start = self.local_head & self.ptr.mask;
            let contiguous = self.ptr.capacity - start;
            let available = contiguous.min(self.local_tail.wrapping_sub(self.local_head));
            assert!(
                len <= available,
                "advancing ({len}) more than available space ({available})"
            );
        }

        let new_head = self.local_head.wrapping_add(len);
        self.store_head(new_head);
        self.local_head = new_head;

        self.ptr.unpark();
    }

    #[inline(always)]
    fn consume(&mut self) -> T {
        let ret = unsafe { self.ptr.get(self.local_head) };
        let new_head = self.local_head.wrapping_add(1);
        self.store_head(new_head);
        self.local_head = new_head;

        self.ptr.unpark();

        ret
    }

    #[inline(always)]
    fn store_head(&self, value: usize) {
        self.ptr.head().store(value, Ordering::Release);
    }

    #[inline(always)]
    fn load_tail(&mut self) {
        self.local_tail = self.ptr.tail().load(Ordering::Acquire);
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
