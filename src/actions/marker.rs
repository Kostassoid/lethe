use roaring::RoaringBitmap;
use std::fmt::Debug;
use winapi::_core::fmt::Formatter;

pub trait BlockMarker {
    fn mark(&mut self, position: u32);
    fn is_marked(&self, position: u32) -> bool;
    fn total_marked(&self) -> u32;
}

impl Debug for dyn BlockMarker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_str("(hidden)")
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
