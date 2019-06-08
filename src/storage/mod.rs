pub mod file;

pub type IoResult<A> = std::io::Result<A>;

pub trait StorageContainer {
    fn size(&self) -> IoResult<u64>;
    fn block_size(&self) -> IoResult<u64>;
    fn position(&mut self) -> IoResult<u64>;

    fn seek(&mut self, position: u64) -> IoResult<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<u64>;
    fn write(&mut self, data: &[u8]) -> IoResult<()>;
    fn sync(&self) -> IoResult<()>;
}

pub trait StorageReference {
    type Container: StorageContainer;
    fn description(&self) -> &str;
    fn to_container(&self) -> IoResult<Box<Self::Container>>;
}

pub trait StorageEnumerator {
    type Reference: StorageReference;
    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Reference>>>;
}