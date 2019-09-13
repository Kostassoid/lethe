use super::sanitization::*;
use super::storage::{StorageAccess, IoResult};
use std::io::{Error, ErrorKind};

#[derive(Debug)]
pub enum Verify {
    No,
    Last,
    All
}

#[derive(Debug)]
pub struct WiperTask {
    pub scheme: Scheme,
    pub verify: Verify,
    pub total_size: u64,
    pub block_size: usize
}

#[derive(Debug, Clone)]
pub struct WiperState {
    pub stage: usize,
    pub at_verification: bool,
    pub position: u64,
}

impl Default for WiperState {
    fn default() -> Self {
        WiperState { stage: 0, at_verification: false, position: 0 }
    }
}

impl WiperTask {
    pub fn new(scheme: Scheme, verify: Verify, total_size: u64, block_size: usize) -> Self {
        WiperTask { 
            scheme, 
            verify, 
            total_size,
            block_size
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WiperEvent {
    StageStarted,
    Progress(u64),
    StageCompleted(IoResult<()>),
    Retrying,
    Aborted,
    Completed(IoResult<()>)
}

pub trait WiperEventReceiver {
    fn handle(&mut self, task: &WiperTask, state: &WiperState, event: WiperEvent) -> ();
}

impl WiperTask {
    pub fn run(self, access: &mut StorageAccess, state: &mut WiperState, frontend: &mut WiperEventReceiver) -> bool {

        let stages = &self.scheme.stages;
        for (i, stage) in stages.iter().enumerate() {

            let have_to_verify = match self.verify {
                Verify::No => false,
                Verify::Last if i + 1 == stages.len() => true,
                Verify::All => true,
                _ => false
            };

            state.stage = i;
            state.position = 0;
            state.at_verification = false;

            loop {

                if !fill(access, &self, state, stage, frontend) {
                    break;
                };

                if !have_to_verify {
                    break;
                }

                state.position = 0;
                state.at_verification = true;

                if !verify(access, &self, state, stage, frontend) {
                    state.at_verification = false;

                    frontend.handle(&self, state, WiperEvent::Retrying)
                } else {
                    break;
                }
            }

        }

        frontend.handle(&self, state, WiperEvent::Completed(Ok(())));

        true
    }
}

fn fill(access: &mut StorageAccess, task: &WiperTask, state: &mut WiperState, stage: &Stage, frontend: &mut WiperEventReceiver) -> bool {

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    frontend.handle(task, state, WiperEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        frontend.handle(task, state, WiperEvent::StageCompleted(Err(err)));
        return false;
    }

    while let Some(chunk) = stream.next() {

        if let Err(err) = access.write(chunk) {
            frontend.handle(task, state, WiperEvent::StageCompleted(Err(err)));
            return false;
        }

        state.position += chunk.len() as u64;
        frontend.handle(task, state, WiperEvent::Progress(state.position));
    };

    if let Err(err) = access.flush() {
        frontend.handle(task, state, WiperEvent::StageCompleted(Err(err)));
        return false;
    }

    frontend.handle(task, state, WiperEvent::StageCompleted(Ok(())));

    true
}

fn verify(access: &mut StorageAccess, task: &WiperTask, state: &mut WiperState, stage: &Stage, frontend: &mut WiperEventReceiver) -> bool {

    frontend.handle(task, state, WiperEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        frontend.handle(task, state, WiperEvent::StageCompleted(Err(err)));
        return false;
    }

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    let mut buf: Vec<u8> = vec![0; task.block_size];

    while let Some(chunk) = stream.next() {
        let b = &mut buf[..chunk.len()];

        if let Err(err) = access.read(b) {
            frontend.handle(task, state, WiperEvent::StageCompleted(Err(err)));
            return false;
        }

        if b != chunk {
            let error = Error::new(ErrorKind::InvalidData, "Verification failed!");
            frontend.handle(task, state, WiperEvent::StageCompleted(Err(Error::from(error.kind()))));
            return false;
        }

        state.position += chunk.len() as u64;
        frontend.handle(task, state, WiperEvent::Progress(state.position));
    }

    frontend.handle(task, state, WiperEvent::StageCompleted(Ok(())));

    true
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::{Cursor, Seek, SeekFrom, Read, Write};
    use assert_matches::*;
    use WiperEvent::*;

    #[test]
    fn test_wiping_happy_path() {

        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver { collected: Vec::new() };

        let task = WiperTask::new(scheme.clone(), Verify::Last, storage.size as u64, block_size);
        let mut state = WiperState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, true);
        assert_eq!(storage.file.get_ref().iter().filter(|x| **x != 0u8).count(), 0);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(_))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(_))));
        assert_matches!(e.next(), Some((_, Completed(_))));
    }

    struct StubReceiver {
        collected: Vec<(WiperState, WiperEvent)>
    }

    impl WiperEventReceiver for StubReceiver {
        fn handle(&mut self, _task: &WiperTask, state: &WiperState, event: WiperEvent) -> () {
            self.collected.push((state.clone(), event));
        }
    }

    struct InMemoryStorage {
        file: Cursor<Vec<u8>>,
        size: usize,
    }

    impl InMemoryStorage {
        fn new(size: usize) -> Self {
            InMemoryStorage { file: Cursor::new(vec![0xff; size]), size }
        }
    }

    impl StorageAccess for InMemoryStorage {
        fn position(&mut self) -> IoResult<u64> {
            self.file.seek(SeekFrom::Current(0))
        }

        fn seek(&mut self, position: u64) -> IoResult<u64> {
            self.file.seek(SeekFrom::Start(position))
        }

        fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
            self.file.read(buffer)
        }

        fn write(&mut self, data: &[u8]) -> IoResult<()> {
            self.file.write_all(data)
        }

        fn flush(&self) -> IoResult<()> {
            Ok(())
        }
    }
}