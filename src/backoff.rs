/// A spinning backoff strategy that spins for a configurable number of iterations
/// before yielding the thread.
///
/// This is used internally by the blocking `send` and `recv` methods to wait for
/// queue space or data to become available. It can also be used directly for custom
/// retry loops with [`try_send`](crate::spsc::Sender::try_send) and
/// [`try_recv`](crate::spsc::Receiver::try_recv).
///
/// The strategy works as follows:
/// 1. For the first `spin_count` calls to [`backoff`](Backoff::backoff), the CPU
///    spins via [`core::hint::spin_loop`].
/// 2. After `spin_count` spins, the counter resets and the thread yields
///    (via [`std::thread::yield_now`] when `std` is available, or
///    [`core::hint::spin_loop`] in `no_std`).
///
/// # Examples
///
/// ```
/// use core::num::NonZeroUsize;
/// use gil::Backoff;
/// use gil::spsc::channel;
///
/// let (mut tx, mut rx) = channel::<i32>(NonZeroUsize::new(16).unwrap());
/// tx.send(42);
///
/// // Custom retry loop using Backoff
/// let mut backoff = Backoff::with_spin_count(64);
/// loop {
///     match rx.try_recv() {
///         Some(val) => {
///             assert_eq!(val, 42);
///             break;
///         }
///         None => backoff.backoff(),
///     }
/// }
/// ```
pub struct Backoff {
    spin_count: u32,
    current: u32,
}

impl Backoff {
    /// Creates a new `Backoff` with the given spin count.
    ///
    /// The backoff will spin for `spin_count` iterations before yielding the
    /// thread on each cycle.
    ///
    /// # Examples
    ///
    /// ```
    /// use gil::Backoff;
    ///
    /// let backoff = Backoff::with_spin_count(128);
    /// ```
    #[inline(always)]
    pub fn with_spin_count(spin_count: u32) -> Self {
        Self {
            spin_count,
            current: 0,
        }
    }

    /// Updates the spin count.
    ///
    /// This does not reset the current spin counter. Call [`reset`](Backoff::reset)
    /// to restart the cycle.
    ///
    /// # Examples
    ///
    /// ```
    /// use gil::Backoff;
    ///
    /// let mut backoff = Backoff::with_spin_count(64);
    /// backoff.set_spin_count(256);
    /// ```
    #[inline(always)]
    pub fn set_spin_count(&mut self, spin_count: u32) {
        self.spin_count = spin_count;
    }

    /// Performs one backoff step.
    ///
    /// If fewer than `spin_count` spins have occurred since the last reset,
    /// this spins the CPU. Once `spin_count` is reached, the counter resets
    /// and the thread yields.
    ///
    /// # Examples
    ///
    /// ```
    /// use gil::Backoff;
    ///
    /// let mut backoff = Backoff::with_spin_count(4);
    /// // First 4 calls spin, the 5th yields and resets
    /// for _ in 0..5 {
    ///     backoff.backoff();
    /// }
    /// ```
    #[inline(always)]
    pub fn backoff(&mut self) {
        if self.current < self.spin_count {
            crate::hint::spin_loop();
            self.current += 1;
        } else {
            self.current = 0;
            crate::thread::yield_now();
        }
    }

    /// Resets the spin counter to zero.
    ///
    /// The next call to [`backoff`](Backoff::backoff) will start spinning from
    /// the beginning.
    ///
    /// # Examples
    ///
    /// ```
    /// use gil::Backoff;
    ///
    /// let mut backoff = Backoff::with_spin_count(4);
    /// for _ in 0..3 {
    ///     backoff.backoff();
    /// }
    /// backoff.reset();
    /// // Starts spinning again from the beginning
    /// backoff.backoff();
    /// ```
    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
    }
}
