use roaring::RoaringBitmap;
use std::fmt::{Debug, Formatter};

pub trait BlockMarker {
    fn mark(&mut self, position: u32);
    fn is_marked(&self, position: u32) -> bool;
    fn total_marked(&self) -> u32;
}

impl Debug for dyn BlockMarker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_str("(block marker)")
    }
}

pub struct RoaringBlockMarker {
    store: RoaringBitmap,
}

impl RoaringBlockMarker {
    pub(crate) fn new() -> RoaringBlockMarker {
        RoaringBlockMarker {
            store: RoaringBitmap::new(),
        }
    }
}

impl BlockMarker for RoaringBlockMarker {
    fn mark(&mut self, position: u32) {
        self.store.insert(position);
    }

    fn is_marked(&self, position: u32) -> bool {
        self.store.contains(position)
    }

    fn total_marked(&self) -> u32 {
        self.store.len() as u32
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_empty_marker() {
        let marker = RoaringBlockMarker::new();

        assert_eq!(0, marker.total_marked());
    }

    #[test]
    fn test_marker_tracking_unique_values() {
        let mut marker = RoaringBlockMarker::new();

        marker.mark(13);

        assert_eq!(1, marker.total_marked());
        assert!(marker.is_marked(13));
        assert!(!marker.is_marked(12));
        assert!(!marker.is_marked(14));

        marker.mark(133);
        assert_eq!(2, marker.total_marked());
        assert!(marker.is_marked(13));
        assert!(marker.is_marked(133));

        marker.mark(13);
        assert_eq!(2, marker.total_marked());
        assert!(marker.is_marked(13));
        assert!(marker.is_marked(133));
    }

    #[test]
    fn test_marker_tracking_edge_values() {
        let mut marker = RoaringBlockMarker::new();

        marker.mark(0);

        assert_eq!(1, marker.total_marked());
        assert!(marker.is_marked(0));
        assert!(!marker.is_marked(1));

        marker.mark(u32::max_value());
        assert_eq!(2, marker.total_marked());
        assert!(marker.is_marked(0));
        assert!(marker.is_marked(u32::max_value()));
    }
}
