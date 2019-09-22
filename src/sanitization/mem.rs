pub fn alloc_aligned_byte_vec(size: usize, align: usize) -> Vec<u8> {
    unsafe {
        let buf_layout = std::alloc::Layout::from_size_align_unchecked(size, align);
        let buf_ptr = std::alloc::alloc(buf_layout);
        Vec::from_raw_parts(buf_ptr, size, size)
    }
}

pub fn fill_byte_slice(buf: &mut [u8], value: u8) {
    // unsafe {
    //     std::libc::memset(
    //         buf.as_mut_ptr() as _,
    //         0,
    //         buf.len()
    //     );
    // };
    buf.iter_mut().map(|x| *x = value).count(); //todo: rewrite
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_aligned_allocation() {
        let size = 65536;
        let align = 4096;

        let buf = alloc_aligned_byte_vec(size, align);

        assert_eq!(buf.len(), size);
        assert_eq!(buf.as_ptr() as usize % align, 0);
    }

    #[test]
    fn test_vec_fill() {
        let mut buf = vec![0x00; 1024];

        fill_byte_slice(&mut buf, 0xff);

        assert_eq!(buf.iter().filter(|x| **x != 0xff).count(), 0);
    }

}
