use core::{mem::MaybeUninit, num::NonZeroUsize, ptr::NonNull};

use crate::{
    spsc,
    sync::atomic::{AtomicUsize, Ordering},
};

/// The sending half of a sharded MPMC channel.
///
/// Each sender is bound to a specific shard. Cloning a sender will attempt to bind the new
/// instance to a different, unused shard.
pub struct Sender<T> {
    inner: spsc::Sender<T>,
    shards: NonNull<spsc::QueuePtr<T>>,
    num_senders: NonNull<AtomicUsize>,
    max_shards: usize,
}

impl<T> Sender<T> {
    /// Attempts to clone the sender.
    ///
    /// Returns `Some(Sender)` if there is an available shard to bind to, otherwise returns `None`.
    pub fn clone(&self) -> Option<Self> {
        unsafe { Self::init(self.shards, self.max_shards, self.num_senders) }
    }

    pub(crate) fn new(shards: NonNull<spsc::QueuePtr<T>>, max_shards: NonZeroUsize) -> Self {
        let num_senders_ptr = Box::into_raw(Box::new(AtomicUsize::new(0)));
        unsafe {
            let num_senders = NonNull::new_unchecked(num_senders_ptr);
            Self::init(shards, max_shards.get(), num_senders).unwrap_unchecked()
        }
    }

    pub(crate) unsafe fn init(
        shards: NonNull<spsc::QueuePtr<T>>,
        max_shards: usize,
        num_senders: NonNull<AtomicUsize>,
    ) -> Option<Self> {
        let num_senders_ref = unsafe { num_senders.as_ref() };
        let next_shard = num_senders_ref.fetch_add(1, Ordering::Relaxed);
        if next_shard >= max_shards {
            num_senders_ref.store(max_shards, Ordering::Relaxed);
            return None;
        }

        let shard_ptr = unsafe { shards.add(next_shard).as_ref() }.clone();
        let inner = spsc::Sender::new(shard_ptr);

        Some(Self {
            inner,
            shards,
            num_senders,
            max_shards,
        })
    }

    /// Sends a value into the channel.
    ///
    /// This method will block (spin) until there is space in the shard's queue.
    pub fn send(&mut self, value: T) {
        self.inner.send(value)
    }

    /// Attempts to send a value into the channel without blocking.
    ///
    /// Returns `Ok(())` if the value was sent, or `Err(value)` if the shard's queue is full.
    pub fn try_send(&mut self, value: T) -> Result<(), T> {
        self.inner.try_send(value)
    }

    /// Returns a slice of the internal write buffer for batched sending.
    pub fn write_buffer(&mut self) -> &mut [MaybeUninit<T>] {
        self.inner.write_buffer()
    }

    /// Commits `len` elements from the write buffer to the channel.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least `len` elements in the write buffer have been initialized.
    pub unsafe fn commit(&mut self, len: usize) {
        unsafe { self.inner.commit(len) }
    }
}

unsafe impl<T> Send for Sender<T> {}
