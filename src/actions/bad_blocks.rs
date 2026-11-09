use roaring::RoaringBitmap;

/// Tracks storage blocks that are known to be inaccessible, by their absolute block number.
///
/// A roaring bitmap keeps the memory footprint negligible even for devices with billions
/// of bad blocks spread across a large address space.
#[derive(Debug, Default)]
pub struct BadBlocks {
    marked: RoaringBitmap,
}

impl BadBlocks {
    pub fn mark(&mut self, block: u32) {
        self.marked.insert(block);
    }

    pub fn contains(&self, block: u32) -> bool {
        self.marked.contains(block)
    }

    pub fn len(&self) -> u64 {
        self.marked.len()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_empty_marker() {
        let marker = BadBlocks::default();

        assert_eq!(0, marker.len());
    }

    #[test]
    fn test_marker_tracking_unique_values() {
        let mut marker = BadBlocks::default();

        marker.mark(13);

        assert_eq!(1, marker.len());
        assert!(marker.contains(13));
        assert!(!marker.contains(12));
        assert!(!marker.contains(14));

        marker.mark(133);
        assert_eq!(2, marker.len());
        assert!(marker.contains(13));
        assert!(marker.contains(133));

        marker.mark(13);
        assert_eq!(2, marker.len());
        assert!(marker.contains(13));
        assert!(marker.contains(133));
    }

    #[test]
    fn test_marker_tracking_edge_values() {
        let mut marker = BadBlocks::default();

        marker.mark(0);

        assert_eq!(1, marker.len());
        assert!(marker.contains(0));
        assert!(!marker.contains(1));

        marker.mark(u32::MAX);
        assert_eq!(2, marker.len());
        assert!(marker.contains(0));
        assert!(marker.contains(u32::MAX));
    }
}
