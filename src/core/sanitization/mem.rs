use std::ptr::slice_from_raw_parts_mut;

pub(crate) struct AlignedBuffer {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl AlignedBuffer {
    pub(crate) fn new(size: usize, align: usize) -> Self {
        unsafe {
            let buf_layout = std::alloc::Layout::from_size_align_unchecked(size, align);
            let buf_ptr = std::alloc::alloc(buf_layout);
            AlignedBuffer {
                ptr: buf_ptr,
                layout: buf_layout,
            }
        }
    }

    pub(crate) fn fill(&mut self, value: u8) -> () {
        unsafe { self.ptr.write_bytes(value, self.layout.size()) }
    }

    pub(crate) fn as_mut_slice(&self) -> &mut [u8] {
        unsafe { &mut *slice_from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr as *mut u8, self.layout) }
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

        assert_eq!(buf.as_mut_slice().len(), size);
        assert_eq!(buf.ptr as usize % align, 0);
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
