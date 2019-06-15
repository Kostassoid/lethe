pub mod nix;

pub type IoResult<A> = std::io::Result<A>;

pub trait StorageAccess {
    fn position(&mut self) -> IoResult<u64>;
    fn seek(&mut self, position: u64) -> IoResult<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<u64>;
    fn write(&mut self, data: &[u8]) -> IoResult<()>;
    fn flush(&self) -> IoResult<()>;
}

#[derive(Debug)]
pub struct StorageDetails {
    pub size: u64,
    pub block_size: u64,
    pub is_readonly: bool
}

pub trait StorageRef {
    type Access: StorageAccess;
    fn id(&self) -> &str;
    fn details(&self) -> &StorageDetails;
    fn access(&self) -> IoResult<Box<Self::Access>>;
}

pub trait StorageEnumerator {
    type Ref: StorageRef;
    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Ref>>>;
}