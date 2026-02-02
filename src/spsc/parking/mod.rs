//! CPU-efficient SPSC variant that parks threads when idle.
//!
//! Same ring buffer as [`super::channel`] but both the sender and receiver
//! will park their thread after exhausting a spin + yield backoff phase.
//! Uses [`atomic_wait`] for cross-platform futex-like wait/wake on separate
//! per-side `AtomicU32` flags — one for the receiver (in `Head`) and one
//! for the sender (in `Tail`).
//!
//! The unpark fast path when nobody is parked is a single `Relaxed` atomic
//! load (after a `SeqCst` fence that prevents Dekker-pattern reordering).
//!
//! # Examples
//!
//! ```
//! use std::thread;
//! use core::num::NonZeroUsize;
//! use gil::spsc::parking;
//!
//! let (mut tx, mut rx) = parking::channel::<usize>(NonZeroUsize::new(1024).unwrap());
//!
//! thread::spawn(move || {
//!     for i in 0..100 {
//!         tx.send(i);
//!     }
//! });
//!
//! for i in 0..100 {
//!     assert_eq!(rx.recv(), i);
//! }
//! ```

use core::num::NonZeroUsize;

pub use self::{receiver::Receiver, sender::Sender};

mod queue;
mod receiver;
mod sender;

/// Creates a new parking SPSC channel.
///
/// See the [module-level documentation](self) for details.
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
pub fn channel<T>(capacity: NonZeroUsize) -> (Sender<T>, Receiver<T>) {
    let queue = queue::QueuePtr::with_size(capacity);
    (Sender::new(queue.clone()), Receiver::new(queue))
}

#[cfg(all(test, not(feature = "loom")))]
mod test {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::thread;

    #[test]
    fn basic_parking() {
        const COUNTS: NonZeroUsize = NonZeroUsize::new(4096).unwrap();
        let (mut tx, mut rx) = channel::<usize>(COUNTS);

        thread::spawn(move || {
            for i in 0..COUNTS.get() << 3 {
                tx.send(i);
            }
        });

        for i in 0..COUNTS.get() << 3 {
            let r = rx.recv();
            assert_eq!(r, i);
        }
    }

    #[test]
    fn try_ops_parking() {
        let (mut tx, mut rx) = channel::<usize>(NonZeroUsize::new(4).unwrap());
        for _ in 0..4 {
            assert!(rx.try_recv().is_none());
        }
        for i in 0..4 {
            tx.try_send(i).unwrap();
        }
        assert!(tx.try_send(5).is_err());

        for i in 0..4 {
            assert_eq!(rx.try_recv(), Some(i));
        }
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn parking_recv_wakes() {
        let (mut tx, mut rx) = channel::<usize>(NonZeroUsize::new(4).unwrap());

        let h = thread::spawn(move || rx.recv());

        // Give the receiver time to park
        thread::sleep(std::time::Duration::from_millis(50));
        tx.send(42);

        assert_eq!(h.join().unwrap(), 42);
    }

    #[test]
    fn parking_send_wakes() {
        // Queue of 1 — sender will park immediately on second send
        let (mut tx, mut rx) = channel::<usize>(NonZeroUsize::new(1).unwrap());
        tx.send(0); // fill the queue

        let h = thread::spawn(move || {
            tx.send(1); // should park until receiver consumes
            tx.send(2);
        });

        thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(rx.recv(), 0);
        assert_eq!(rx.recv(), 1);
        assert_eq!(rx.recv(), 2);

        h.join().unwrap();
    }
}
