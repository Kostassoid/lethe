use crate::sanitization::*;
use crate::storage::StorageAccess;
use std::io::{Error, ErrorKind};
use std::rc::Rc;

#[derive(Debug)]
pub enum Verify {
    No,
    Last,
    All
}

#[derive(Debug)]
pub struct WipeTask {
    pub scheme: Scheme,
    pub verify: Verify,
    pub total_size: u64,
    pub block_size: usize
}

#[derive(Debug, Clone)]
pub struct WipeState {
    pub stage: usize,
    pub at_verification: bool,
    pub position: u64,
}

impl Default for WipeState {
    fn default() -> Self {
        WipeState { stage: 0, at_verification: false, position: 0 }
    }
}

impl WipeTask {
    pub fn new(scheme: Scheme, verify: Verify, total_size: u64, block_size: usize) -> Self {
        WipeTask { 
            scheme, 
            verify, 
            total_size,
            block_size
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WipeEvent {
    Started,
    StageStarted,
    Progress(u64),
    StageCompleted(Option<Rc<Error>>),
    Retrying,
    Aborted,
    Completed(Option<Rc<Error>>),
    Fatal(Rc<Error>)
}

pub trait WipeEventReceiver {
    fn handle(&mut self, task: &WipeTask, state: &WipeState, event: WipeEvent) -> ();
}

impl WipeTask {
    pub fn run(self, access: &mut StorageAccess, state: &mut WipeState, frontend: &mut WipeEventReceiver) -> bool {

        frontend.handle(&self, state, WipeEvent::Started);

        let stages = &self.scheme.stages;

        let mut wipe_error = None;

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

            let stage_error = loop {

                if let Some(err) = fill(access, &self, state, stage, frontend) {
                    break Some(err);
                };

                if !have_to_verify {
                    break None;
                }

                state.position = 0;
                state.at_verification = true;

                if verify(access, &self, state, stage, frontend).is_some() {
                    state.at_verification = false;
                    state.position = 0; //todo: optimize by starting from latest good position
                    frontend.handle(&self, state, WipeEvent::Retrying)
                } else {
                    break None;
                }
            };

            if stage_error.is_some() {
                wipe_error = stage_error;
                break;
            };
        };

        let result = wipe_error.is_none();
        frontend.handle(&self, state, WipeEvent::Completed(wipe_error));

        result
    }
}

fn fill(access: &mut StorageAccess, task: &WipeTask, state: &mut WipeState, stage: &Stage, frontend: &mut WipeEventReceiver) -> Option<Rc<Error>> {

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    frontend.handle(task, state, WipeEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        let err_rc = Rc::from(err);
        frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));
        return Some(Rc::clone(&err_rc));
    }

    while let Some(chunk) = stream.next() {

        if let Err(err) = access.write(chunk) {
            let err_rc = Rc::from(err);
            frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));
            return Some(Rc::clone(&err_rc));
        }

        state.position += chunk.len() as u64;
        frontend.handle(task, state, WipeEvent::Progress(state.position));
    };

    if let Err(err) = access.flush() {
        let err_rc = Rc::from(err);
        frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));
        return Some(Rc::clone(&err_rc));
    }

    frontend.handle(task, state, WipeEvent::StageCompleted(None));

    None
}

fn verify(access: &mut StorageAccess, task: &WipeTask, state: &mut WipeState, stage: &Stage, frontend: &mut WipeEventReceiver) -> Option<Rc<Error>> {

    frontend.handle(task, state, WipeEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        let err_rc = Rc::from(err);
        frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));
        return Some(Rc::clone(&err_rc));
    }

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    let mut buf: Vec<u8> = vec![0; task.block_size];

    while let Some(chunk) = stream.next() {
        let b = &mut buf[..chunk.len()];

        if let Err(err) = access.read(b) {
            let err_rc = Rc::from(err);
            frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));
            return Some(Rc::clone(&err_rc));
        }

        if b != chunk {
            let error = Rc::from(Error::new(ErrorKind::InvalidData, "Verification failed!"));
            frontend.handle(task, state, WipeEvent::StageCompleted(Some(Rc::clone(&error))));
            return Some(Rc::clone(&error));
        }

        state.position += chunk.len() as u64;
        frontend.handle(task, state, WipeEvent::Progress(state.position));
    }

    frontend.handle(task, state, WipeEvent::StageCompleted(None));

    None
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::storage::IoResult;
    use std::io::{Cursor, Seek, SeekFrom, Read, Write};
    use assert_matches::*;
    use WipeEvent::*;

    #[test]
    fn test_wiping_happy_path() {

        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        let task = WipeTask::new(scheme.clone(), Verify::Last, storage.size as u64, block_size);
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, true);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));

        assert_eq!(storage.file.get_ref().iter().filter(|x| **x != 0u8).count(), 0);
    }

    #[test]
    fn test_wiping_fill_failure() {

        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_after_any(50000);

        let task = WipeTask::new(scheme.clone(), Verify::Last, storage.size as u64, block_size);
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, false);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StageCompleted(Some(_)))));
        assert_matches!(e.next(), Some((_, Completed(Some(_)))));

        assert_eq!(storage.file.get_ref().iter().filter(|x| **x != 0u8).count(), 100000 - 32768);
    }

    #[test]
    fn test_wiping_validation_failure() {

        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_after_any(150000);

        let task = WipeTask::new(scheme.clone(), Verify::Last, storage.size as u64, block_size);
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, true);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StageCompleted(Some(_)))));
        assert_matches!(e.next(), Some((_, Retrying)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0/*32768*/);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));

        assert_eq!(storage.file.get_ref().iter().filter(|x| **x != 0u8).count(), 0);
    }

    struct StubReceiver {
        collected: Vec<(WipeState, WipeEvent)>
    }

    impl StubReceiver {
        pub fn new() -> Self {
            StubReceiver { collected: Vec::new() }
        }
    }

    impl WipeEventReceiver for StubReceiver {
        fn handle(&mut self, _task: &WipeTask, state: &WipeState, event: WipeEvent) -> () {
            self.collected.push((state.clone(), event));
        }
    }

    struct InMemoryStorage {
        file: Cursor<Vec<u8>>,
        size: usize,
        total_written: usize,
        total_read: usize,
        failures: Vec<(usize)>
    }

    impl InMemoryStorage {
        fn new(size: usize) -> Self {
            InMemoryStorage { 
                file: Cursor::new(vec![0xff; size]), 
                size,
                total_written: 0,
                total_read: 0,
                failures: Vec::new()
            }
        }

        fn fail_after_any(&mut self, amount: usize) -> () {
            self.failures.push(amount);
            self.failures.sort();
        }

        fn check_and_fail(&mut self, amount_read: usize, amount_written: usize) -> IoResult<()> {
            let old_total = self.total_read + self.total_written;

            self.total_read += amount_read;
            self.total_written += amount_written;

            match self.failures.iter().find(|x| **x >= old_total) {
                Some(v) if old_total + amount_read + amount_written > *v => 
                    Err(Error::from(ErrorKind::Interrupted)), 
                _ => 
                    Ok(())
            }
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
            self.check_and_fail(buffer.len(), 0)?;
            self.file.read(buffer)
        }

        fn write(&mut self, data: &[u8]) -> IoResult<()> {
            self.check_and_fail(0, data.len())?;
            self.file.write_all(data)
        }

        fn flush(&self) -> IoResult<()> {
            Ok(())
        }
    }
}