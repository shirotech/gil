use core::marker::PhantomData;

use crate::{
    atomic::{AtomicUsize, Ordering},
    cell::{Cell, CellPtr},
    padded::Padded,
};

#[derive(Default)]
#[repr(C)]
pub(crate) struct Head {
    head: Padded<AtomicUsize>,
}

pub(crate) struct GetInit;
impl<T> crate::DropInitItems<Head, (), Cell<T>> for GetInit {
    unsafe fn drop_init_items(
        head: core::ptr::NonNull<Head>,
        _tail: core::ptr::NonNull<()>,
        capacity: usize,
        at: impl Fn(usize) -> core::ptr::NonNull<Cell<T>>,
    ) {
        if !core::mem::needs_drop::<T>() {
            return;
        }

        let head = unsafe { _field!(Head, head, head.value, AtomicUsize).as_ref() }
            .load(Ordering::Relaxed);

        for i in 0..capacity {
            let idx = head.wrapping_add(i);
            let cell = CellPtr::from(at(idx));
            let epoch = cell.epoch().load(Ordering::Relaxed);
            if epoch > idx && (epoch & (capacity - 1)) == (idx & (capacity - 1)) {
                unsafe { cell.drop_in_place() };
            }
        }
    }
}

pub(crate) struct Initializer<T> {
    _marker: PhantomData<T>,
}

impl<T> crate::Initializer for Initializer<T> {
    type Item = Cell<T>;

    fn initialize(idx: usize, cell: &mut Self::Item) {
        cell.epoch.store(idx, Ordering::Relaxed);
    }
}

pub(crate) type Queue = crate::Queue<Head, ()>;
pub(crate) type QueuePtr<T> = crate::QueuePtr<Head, (), Cell<T>, GetInit>;

impl<T> QueuePtr<T> {
    #[inline(always)]
    pub(crate) fn head(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, head.head.value, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) fn cell_at(&self, index: usize) -> CellPtr<T> {
        self.at(index).into()
    }
}
