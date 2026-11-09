use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::ptr::NonNull;

/// A heap-allocated byte buffer aligned to a chosen boundary.
///
/// Alignment beyond what `Vec<u8>` guarantees is required for direct (unbuffered) I/O:
/// the kernel rejects transfers through buffers whose addresses are not aligned to the
/// storage device's logical block size.
pub(crate) struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBuffer {
    pub(crate) fn new(size: usize, align: usize) -> Self {
        assert!(size > 0, "buffer size must be non-zero");
        let layout = Layout::from_size_align(size, align).expect("invalid buffer layout");

        // SAFETY: `layout` was produced by the checked constructor, so it has a non-zero
        // size and a valid (power-of-two) alignment.
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).unwrap_or_else(|| handle_alloc_error(layout));

        AlignedBuffer { ptr, layout }
    }

    pub(crate) fn fill(&mut self, value: u8) {
        self.as_mut_slice().fill(value);
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is valid for reads of `layout.size()` bytes for the lifetime of
        // `self`, and `layout.size()` bytes were allocated.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.layout.size()) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` is valid for reads and writes of `layout.size()` bytes for the
        // lifetime of `self`, and `layout.size()` bytes were allocated.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }

    pub(crate) fn len(&self) -> usize {
        self.layout.size()
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: `ptr` was allocated with exactly `layout`.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_aligned_allocation() {
        let size = 65536;
        let align = 4096;

        let buf = AlignedBuffer::new(size, align);

        assert_eq!(buf.as_slice().len(), size);
        assert_eq!(buf.ptr.as_ptr() as usize % align, 0);
    }

    #[test]
    fn test_vec_fill() {
        let mut buf = AlignedBuffer::new(1024, 1024);

        buf.fill(0xff);
        assert_eq!(buf.as_mut_slice().iter().filter(|x| **x != 0xff).count(), 0);

        buf.fill(0x11);
        assert_eq!(buf.as_mut_slice().iter().filter(|x| **x != 0x11).count(), 0);
    }
}
