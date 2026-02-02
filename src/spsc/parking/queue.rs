use core::ptr::NonNull;

use crate::{
    atomic::{AtomicUsize, Ordering},
    padded::Padded,
};

const NOT_PARKED: u32 = 0;
const PARKED: u32 = 1;

#[repr(C)]
pub(crate) struct Head {
    head: Padded<AtomicUsize>,
    #[cfg(not(feature = "loom"))]
    receiver_parked: Padded<core::sync::atomic::AtomicU32>,
    #[cfg(feature = "loom")]
    receiver_parked: Padded<loom::sync::atomic::AtomicBool>,
    #[cfg(feature = "loom")]
    receiver_thread: core::mem::MaybeUninit<loom::thread::Thread>,
}

#[repr(C)]
pub(crate) struct Tail {
    tail: Padded<AtomicUsize>,
    #[cfg(not(feature = "loom"))]
    sender_parked: Padded<core::sync::atomic::AtomicU32>,
    #[cfg(feature = "loom")]
    sender_parked: Padded<loom::sync::atomic::AtomicBool>,
    #[cfg(feature = "loom")]
    sender_thread: core::mem::MaybeUninit<loom::thread::Thread>,
}

impl Default for Head {
    fn default() -> Self {
        Self {
            head: Default::default(),
            #[cfg(not(feature = "loom"))]
            receiver_parked: Padded::new(core::sync::atomic::AtomicU32::new(NOT_PARKED)),
            #[cfg(feature = "loom")]
            receiver_parked: Default::default(),
            #[cfg(feature = "loom")]
            receiver_thread: core::mem::MaybeUninit::uninit(),
        }
    }
}

impl Default for Tail {
    fn default() -> Self {
        Self {
            tail: Default::default(),
            #[cfg(not(feature = "loom"))]
            sender_parked: Padded::new(core::sync::atomic::AtomicU32::new(NOT_PARKED)),
            #[cfg(feature = "loom")]
            sender_parked: Default::default(),
            #[cfg(feature = "loom")]
            sender_thread: core::mem::MaybeUninit::uninit(),
        }
    }
}

pub(crate) struct GetInit;

impl<T> crate::DropInitItems<Head, Tail, T> for GetInit {
    unsafe fn drop_init_items(
        head: NonNull<Head>,
        tail: NonNull<Tail>,
        _capacity: usize,
        at: impl Fn(usize) -> NonNull<T>,
    ) {
        if !core::mem::needs_drop::<T>() {
            return;
        }

        let (head, tail) = unsafe {
            let head = _field!(Head, head, head.value, AtomicUsize)
                .as_ref()
                .load(Ordering::Relaxed);
            let tail = _field!(Tail, tail, tail.value, AtomicUsize)
                .as_ref()
                .load(Ordering::Relaxed);
            (head, tail)
        };
        let len = tail.wrapping_sub(head);

        for i in 0..len {
            let idx = head.wrapping_add(i);
            unsafe { at(idx).drop_in_place() };
        }
    }
}

pub(crate) type QueuePtr<T> = crate::QueuePtr<Head, Tail, T, GetInit>;
type Queue = crate::Queue<Head, Tail>;

/// Dekker-pattern ordering for the parking protocol.
///
/// Each side stores to its own variable then loads the other's:
///   Sender:   store(tail)          → load(receiver_parked)
///   Receiver: store(receiver_parked) → load(tail)
///   (and symmetrically for head / sender_parked)
///
/// All four operations in each pair must use `SeqCst` so they participate
/// in a single total order. Without this the compiler may reorder the load
/// before the store, missing a concurrent park and causing deadlock.
/// On arm64 `SeqCst` store/load emit the same `stlr`/`ldar` as
/// Release/Acquire — the constraint is purely on the compiler.
impl<T> QueuePtr<T> {
    #[inline(always)]
    pub(crate) fn head(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, head.head.value, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) fn tail(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, tail.tail.value, AtomicUsize).as_ref() }
    }

    // ── receiver parking ───────────────────────────────────────────

    #[inline(always)]
    pub(crate) fn set_receiver_parked(&self) {
        unsafe {
            #[cfg(feature = "loom")]
            {
                let slot = _field!(
                    Queue,
                    self.ptr,
                    head.receiver_thread,
                    core::mem::MaybeUninit<loom::thread::Thread>
                )
                .as_mut();
                slot.write(loom::thread::current());
            }

            #[cfg(not(feature = "loom"))]
            _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref()
            .store(PARKED, Ordering::SeqCst);

            #[cfg(feature = "loom")]
            _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref()
            .store(true, Ordering::SeqCst);
        }
    }

    #[inline(always)]
    pub(crate) fn clear_receiver_parked(&self) {
        unsafe {
            #[cfg(not(feature = "loom"))]
            _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref()
            .store(NOT_PARKED, Ordering::Release);

            #[cfg(feature = "loom")]
            _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref()
            .store(false, Ordering::Release);
        }
    }

    #[inline(always)]
    pub(crate) fn wait_as_receiver(&self) {
        #[cfg(not(feature = "loom"))]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref();
            atomic_wait::wait(flag, PARKED);
        }
        #[cfg(feature = "loom")]
        {
            loom::thread::park();
            self.clear_receiver_parked();
        }
    }

    #[inline(always)]
    pub(crate) fn try_unpark_receiver(&self) {
        #[cfg(not(feature = "loom"))]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref();
            if flag.load(Ordering::SeqCst) == PARKED {
                flag.store(NOT_PARKED, Ordering::Release);
                atomic_wait::wake_one(flag);
            }
        }
        #[cfg(feature = "loom")]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                head.receiver_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref();
            if flag.load(Ordering::SeqCst) && flag.swap(false, Ordering::AcqRel) {
                let slot = _field!(
                    Queue,
                    self.ptr,
                    head.receiver_thread,
                    core::mem::MaybeUninit<loom::thread::Thread>
                )
                .as_ref();
                slot.assume_init_ref().unpark();
            }
        }
    }

    // ── sender parking ─────────────────────────────────────────────

    #[inline(always)]
    pub(crate) fn set_sender_parked(&self) {
        unsafe {
            #[cfg(feature = "loom")]
            {
                let slot = _field!(
                    Queue,
                    self.ptr,
                    tail.sender_thread,
                    core::mem::MaybeUninit<loom::thread::Thread>
                )
                .as_mut();
                slot.write(loom::thread::current());
            }

            #[cfg(not(feature = "loom"))]
            _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref()
            .store(PARKED, Ordering::SeqCst);

            #[cfg(feature = "loom")]
            _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref()
            .store(true, Ordering::SeqCst);
        }
    }

    #[inline(always)]
    pub(crate) fn clear_sender_parked(&self) {
        unsafe {
            #[cfg(not(feature = "loom"))]
            _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref()
            .store(NOT_PARKED, Ordering::Release);

            #[cfg(feature = "loom")]
            _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref()
            .store(false, Ordering::Release);
        }
    }

    #[inline(always)]
    pub(crate) fn wait_as_sender(&self) {
        #[cfg(not(feature = "loom"))]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref();
            atomic_wait::wait(flag, PARKED);
        }
        #[cfg(feature = "loom")]
        {
            loom::thread::park();
            self.clear_sender_parked();
        }
    }

    #[inline(always)]
    pub(crate) fn try_unpark_sender(&self) {
        #[cfg(not(feature = "loom"))]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                core::sync::atomic::AtomicU32
            )
            .as_ref();
            if flag.load(Ordering::SeqCst) == PARKED {
                flag.store(NOT_PARKED, Ordering::Release);
                atomic_wait::wake_one(flag);
            }
        }
        #[cfg(feature = "loom")]
        unsafe {
            let flag = _field!(
                Queue,
                self.ptr,
                tail.sender_parked.value,
                loom::sync::atomic::AtomicBool
            )
            .as_ref();
            if flag.load(Ordering::SeqCst) && flag.swap(false, Ordering::AcqRel) {
                let slot = _field!(
                    Queue,
                    self.ptr,
                    tail.sender_thread,
                    core::mem::MaybeUninit<loom::thread::Thread>
                )
                .as_ref();
                slot.assume_init_ref().unpark();
            }
        }
    }
}
