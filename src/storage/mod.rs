pub mod file;

pub type IoResult<A> = std::io::Result<A>;

pub trait StorageAccess {
    fn position(&mut self) -> IoResult<u64>;
    fn seek(&mut self, position: u64) -> IoResult<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<u64>;
    fn write(&mut self, data: &[u8]) -> IoResult<()>;
    fn sync(&self) -> IoResult<()>;
}

pub trait StorageDetails {
    type Access: StorageAccess;
    fn name(&self) -> &str;
    fn size(&self) -> IoResult<u64>;
    fn block_size(&self) -> IoResult<u64>;
    fn is_readonly(&self) -> bool;
    fn is_ok(&self) -> bool;
    fn access(&self) -> IoResult<Box<Self::Access>>;
}

pub trait StorageEnumerator {
    type Details: StorageDetails;
    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Details>>>;
}