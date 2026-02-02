use core::ptr::NonNull;

use crate::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    padded::Padded,
    std_cell::UnsafeCell,
    thread::Thread,
};

#[derive(Default)]
#[repr(C)]
pub(crate) struct Head {
    head: Padded<AtomicUsize>,
    parked: Padded<AtomicBool>,
    parked_thread: UnsafeCell<Option<Thread>>,
}

#[derive(Default)]
#[repr(C)]
pub(crate) struct Tail {
    tail: Padded<AtomicUsize>,
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

impl<T> QueuePtr<T> {
    #[inline(always)]
    pub(crate) fn head(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, head.head.value, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) fn tail(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, tail.tail.value, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) fn store_thread(&self) {
        unsafe {
            let cell = _field!(
                Queue,
                self.ptr,
                head.parked_thread,
                core::cell::UnsafeCell<Option<std::thread::Thread>>
            );
            *(*cell.as_ptr()).get() = Some(std::thread::current());
        }
    }

    #[inline(always)]
    pub(crate) fn set_parked(&self, parked: bool) {
        unsafe {
            _field!(Queue, self.ptr, head.parked, AtomicBool)
                .as_ref()
                .store(parked, Ordering::Release);
        }
    }

    /// Unparks the parked thread (sender or receiver) if one is parked.
    ///
    /// Fast path is a single `Relaxed` load. Only escalates to a
    /// read-modify-write when someone is actually parked.
    #[inline(always)]
    pub(crate) fn unpark(&self) {
        unsafe {
            let flag = _field!(Queue, self.ptr, head.parked, AtomicBool).as_ref();
            if flag.load(Ordering::Relaxed) && flag.swap(false, Ordering::AcqRel) {
                let cell = _field!(
                    Queue,
                    self.ptr,
                    head.parked_thread,
                    core::cell::UnsafeCell<Option<std::thread::Thread>>
                );
                if let Some(thread) = &*(*cell.as_ptr()).get() {
                    thread.unpark();
                }
            }
        }
    }
}
