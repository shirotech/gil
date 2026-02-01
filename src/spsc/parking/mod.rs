//! CPU-efficient SPSC variant that parks the thread when idle.
//!
//! This uses the same ring buffer design as [`super::channel`] but adds
//! thread parking: after a configurable spin + yield phase the receiver
//! thread is parked via [`std::thread::park`], and the sender calls
//! [`Thread::unpark`](std::thread::Thread::unpark) after each write.
//!
//! The trade-off compared to the regular SPSC channel is higher wakeup
//! latency (microseconds instead of nanoseconds) in exchange for near-zero
//! CPU usage when the queue is idle.
//!
//! The sender's fast path when no receiver is parked is a single
//! `Relaxed` atomic load — the same cost as a regular memory read on x86.
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
}
