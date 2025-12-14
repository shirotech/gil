use std::{
    mem::{MaybeUninit, offset_of},
    ptr::NonNull,
};

use crate::atomic::AtomicUsize;

macro_rules! _field {
    ($ptr:expr, $($path:tt).+) => {
        $ptr.byte_add(offset_of!(Cell<T>, $($path).+))
    };

    ($ptr:expr, $($path:tt).+, $field_ty:ty) => {
        $ptr.byte_add(offset_of!(Cell<T>, $($path).+)).cast::<$field_ty>()
    };
}

#[repr(align(64))]
#[cfg_attr(all(target_arch = "aarch64", target_os = "macos"), repr(align(128)))]
pub(crate) struct Cell<T> {
    pub(crate) epoch: AtomicUsize,
    pub(crate) data: MaybeUninit<T>,
}

pub(crate) struct CellPtr<T> {
    ptr: NonNull<Cell<T>>,
}

impl<T> CellPtr<T> {
    /// # Safety
    /// The value must be initialised correctly at this `index`
    #[inline(always)]
    pub(crate) unsafe fn get(&self) -> T {
        unsafe { _field!(self.ptr, data, T).read() }
    }

    #[inline(always)]
    pub(crate) fn set(&self, value: T) {
        unsafe { _field!(self.ptr, data, T).write(value) }
    }

    #[inline(always)]
    pub(crate) fn epoch(&self) -> &AtomicUsize {
        unsafe { _field!(self.ptr, epoch, AtomicUsize).as_ref() }
    }

    #[inline(always)]
    pub(crate) unsafe fn drop_in_place(&self) {
        if std::mem::needs_drop::<T>() {
            unsafe {
                std::ptr::drop_in_place(_field!(self.ptr, data, T).as_ptr());
            }
        }
    }
}

impl<T> From<NonNull<Cell<T>>> for CellPtr<T> {
    fn from(value: NonNull<Cell<T>>) -> Self {
        Self { ptr: value }
    }
}
