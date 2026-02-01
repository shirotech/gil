use core::{mem::MaybeUninit, num::NonZeroUsize, ptr::NonNull};

use crate::{
    Box,
    spsc::{self, shards::ShardsPtr},
    sync::atomic::{AtomicUsize, Ordering},
};

/// The sending half of a sharded MPMC channel.
///
/// Each sender is bound to a specific shard. Cloning a sender will attempt to bind the new
/// instance to a different, unused shard.
///
/// # Examples
///
/// ```
/// use core::num::NonZeroUsize;
/// use gil::mpmc::sharded::channel;
///
/// let (mut tx, mut rx) = channel::<i32>(
///     NonZeroUsize::new(2).unwrap(),
///     NonZeroUsize::new(16).unwrap(),
/// );
///
/// let mut tx2 = tx.try_clone().expect("shard available");
/// tx.send(1);
/// tx2.send(2);
///
/// let mut values = [rx.recv(), rx.recv()];
/// values.sort();
/// assert_eq!(values, [1, 2]);
/// ```
pub struct Sender<T> {
    inner: spsc::Sender<T>,
    shards: ShardsPtr<T>,
    num_senders: NonNull<AtomicUsize>,
    alive_senders: NonNull<AtomicUsize>,
    max_shards: usize,
}

impl<T> Sender<T> {
    /// Attempts to clone the sender.
    ///
    /// Returns `Some(Sender)` if there is an available shard to bind to, or `None` if
    /// all shards are already occupied.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::mpmc::sharded::channel;
    ///
    /// let (tx, rx) = channel::<i32>(
    ///     NonZeroUsize::new(2).unwrap(),
    ///     NonZeroUsize::new(16).unwrap(),
    /// );
    ///
    /// let tx2 = tx.try_clone().expect("shard available");
    ///
    /// // Only 2 shards, so the third clone fails
    /// assert!(tx.try_clone().is_none());
    /// ```
    pub fn try_clone(&self) -> Option<Self> {
        unsafe {
            Self::init(
                self.shards.clone(),
                self.max_shards,
                self.num_senders,
                self.alive_senders,
            )
        }
    }

    pub(super) fn new(shards: ShardsPtr<T>, max_shards: NonZeroUsize) -> Self {
        let num_senders_ptr = Box::into_raw(Box::new(AtomicUsize::new(0)));
        let alive_senders_ptr = Box::into_raw(Box::new(AtomicUsize::new(0)));
        unsafe {
            let num_senders = NonNull::new_unchecked(num_senders_ptr);
            let alive_senders = NonNull::new_unchecked(alive_senders_ptr);
            Self::init(shards, max_shards.get(), num_senders, alive_senders).unwrap_unchecked()
        }
    }

    unsafe fn init(
        shards: ShardsPtr<T>,
        max_shards: usize,
        num_senders: NonNull<AtomicUsize>,
        alive_senders: NonNull<AtomicUsize>,
    ) -> Option<Self> {
        let num_senders_ref = unsafe { num_senders.as_ref() };
        let next_shard = num_senders_ref.fetch_add(1, Ordering::Relaxed);
        if next_shard >= max_shards {
            num_senders_ref.store(max_shards, Ordering::Relaxed);
            return None;
        }

        // AcqRel because can't have this before num_senders is done
        unsafe { alive_senders.as_ref() }.fetch_add(1, Ordering::AcqRel);

        let shard_ptr = shards.clone_queue_ptr(next_shard);
        let inner = spsc::Sender::new(shard_ptr);

        Some(Self {
            inner,
            shards,
            num_senders,
            alive_senders,
            max_shards,
        })
    }

    /// Sends a value into the channel.
    ///
    /// This method will block (spin) until there is space in the shard's queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::mpmc::sharded::channel;
    ///
    /// let (mut tx, mut rx) = channel::<i32>(
    ///     NonZeroUsize::new(1).unwrap(),
    ///     NonZeroUsize::new(16).unwrap(),
    /// );
    /// tx.send(42);
    /// assert_eq!(rx.recv(), 42);
    /// ```
    pub fn send(&mut self, value: T) {
        self.inner.send(value)
    }

    /// Attempts to send a value into the channel without blocking.
    ///
    /// Returns `Ok(())` if the value was sent, or `Err(value)` if the shard's queue is full.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::mpmc::sharded::channel;
    ///
    /// let (mut tx, mut rx) = channel::<i32>(
    ///     NonZeroUsize::new(1).unwrap(),
    ///     NonZeroUsize::new(2).unwrap(),
    /// );
    ///
    /// assert!(tx.try_send(1).is_ok());
    /// assert!(tx.try_send(2).is_ok());
    /// assert_eq!(tx.try_send(3), Err(3));
    /// ```
    pub fn try_send(&mut self, value: T) -> Result<(), T> {
        self.inner.try_send(value)
    }

    /// Returns a mutable slice of the internal write buffer for batched sending.
    ///
    /// After writing to the buffer, call [`commit`](Sender::commit) to make the items
    /// visible to receivers.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::mpmc::sharded::channel;
    ///
    /// let (mut tx, mut rx) = channel::<usize>(
    ///     NonZeroUsize::new(1).unwrap(),
    ///     NonZeroUsize::new(128).unwrap(),
    /// );
    ///
    /// let buf = tx.write_buffer();
    /// buf[0].write(10);
    /// buf[1].write(20);
    /// unsafe { tx.commit(2) };
    ///
    /// assert_eq!(rx.recv(), 10);
    /// assert_eq!(rx.recv(), 20);
    /// ```
    pub fn write_buffer(&mut self) -> &mut [MaybeUninit<T>] {
        self.inner.write_buffer()
    }

    /// Commits `len` elements from the write buffer to the channel.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least `len` elements in the write buffer have been initialized.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::num::NonZeroUsize;
    /// use gil::mpmc::sharded::channel;
    ///
    /// let (mut tx, mut rx) = channel::<usize>(
    ///     NonZeroUsize::new(1).unwrap(),
    ///     NonZeroUsize::new(128).unwrap(),
    /// );
    ///
    /// let buf = tx.write_buffer();
    /// buf[0].write(42);
    /// unsafe { tx.commit(1) };
    ///
    /// assert_eq!(rx.recv(), 42);
    /// ```
    pub unsafe fn commit(&mut self, len: usize) {
        unsafe { self.inner.commit(len) }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe {
            if self.alive_senders.as_ref().fetch_sub(1, Ordering::AcqRel) == 1 {
                _ = Box::from_raw(self.num_senders.as_ptr());
                _ = Box::from_raw(self.alive_senders.as_ptr());
            }
        }
    }
}

unsafe impl<T> Send for Sender<T> {}
