use crate::{
    atomic::{AtomicUsize, Ordering},
    cell::{Cell, CellPtr},
    padded::Padded,
};

#[derive(Default)]
#[repr(C)]
pub(crate) struct Tail {
    tail: Padded<AtomicUsize>,
}

pub(crate) struct GetInit;
impl<T> crate::GetInit<(), Tail, Cell<T>> for GetInit {
    unsafe fn get_init(
        _head: core::ptr::NonNull<()>,
        tail: core::ptr::NonNull<Tail>,
        capaity: usize,
        at: impl Fn(usize) -> core::ptr::NonNull<Cell<T>>,
    ) -> impl Iterator<Item = usize> {
        let tail = unsafe { _field!(Tail, tail, tail.value, AtomicUsize).as_ref() }
            .load(Ordering::Relaxed);

        (1..=capaity).filter_map(move |i| {
            let idx = tail.wrapping_sub(i);
            let cell = at(idx);
            let epoch = unsafe { _field!(Cell<T>, cell, epoch, AtomicUsize).as_ref() }
                .load(Ordering::Relaxed);
            (epoch == idx.wrapping_add(1)).then_some(idx)
        })
    }
}

pub(crate) type Queue = crate::Queue<(), Tail>;
pub(crate) type QueuePtr<T> = crate::QueuePtr<(), Tail, Cell<T>, GetInit>;

impl<T> QueuePtr<T> {
    #[inline(always)]
    pub(crate) fn tail(&self) -> &AtomicUsize {
        unsafe { _field!(Queue, self.ptr, tail.tail.value, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) fn cell_at(&self, index: usize) -> CellPtr<T> {
        self.at(index).into()
    }
}
