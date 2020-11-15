use crate::sanitization::mem::*;
use crate::sanitization::*;
use crate::storage::StorageAccess;
use std::rc::Rc;

#[derive(Debug)]
pub enum Verify {
    No,
    Last,
    All,
}

#[derive(Debug)]
pub struct WipeTask {
    pub scheme: Scheme,
    pub verify: Verify,
    pub total_size: u64,
    pub block_size: usize,
}

#[derive(Debug, Clone)]
pub struct WipeState {
    pub stage: usize,
    pub at_verification: bool,
    pub position: u64,
    pub retries_left: u32,
}

impl Default for WipeState {
    fn default() -> Self {
        WipeState {
            stage: 0,
            at_verification: false,
            position: 0,
            retries_left: 0,
        }
    }
}

impl WipeTask {
    pub fn new(scheme: Scheme, verify: Verify, total_size: u64, block_size: usize) -> Self {
        WipeTask {
            scheme,
            verify,
            total_size,
            block_size,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WipeEvent {
    Started,
    StageStarted,
    Progress(u64),
    StageCompleted(Option<Rc<anyhow::Error>>),
    Retrying,
    Aborted,
    Completed(Option<Rc<anyhow::Error>>),
    Fatal(Rc<anyhow::Error>),
}

pub trait WipeEventReceiver {
    fn handle(&mut self, task: &WipeTask, state: &WipeState, event: WipeEvent) -> ();
}

impl WipeTask {
    pub fn run(
        self,
        access: &mut dyn StorageAccess,
        state: &mut WipeState,
        frontend: &mut dyn WipeEventReceiver,
    ) -> bool {
        frontend.handle(&self, state, WipeEvent::Started);

        let stages = &self.scheme.stages;

        let mut wipe_error = None;

        for (i, stage) in stages.iter().enumerate() {
            let have_to_verify = match self.verify {
                Verify::No => false,
                Verify::Last if i + 1 == stages.len() => true,
                Verify::All => true,
                _ => false,
            };

            state.stage = i;
            state.position = 0;
            state.at_verification = false;

            let stage_error = loop {
                let watermark = state.position;

                match fill(access, &self, state, stage, frontend) {
                    Some(_) if state.retries_left > 0 => {
                        state.retries_left -= 1;
                        frontend.handle(&self, state, WipeEvent::Retrying);
                        continue;
                    }
                    Some(err) => break Some(err),
                    None => {}
                };

                if !have_to_verify {
                    break None;
                }

                state.position = watermark;
                state.at_verification = true;

                match verify(access, &self, state, stage, frontend) {
                    Some(_) if state.retries_left > 0 => {
                        state.retries_left -= 1;
                        state.at_verification = false;
                        frontend.handle(&self, state, WipeEvent::Retrying);
                    }
                    Some(err) => break Some(err),
                    None => break None,
                }
            };

            if stage_error.is_some() {
                wipe_error = stage_error;
                break;
            };
        }

        let result = wipe_error.is_none();
        frontend.handle(&self, state, WipeEvent::Completed(wipe_error));

        result
    }
}

fn fill(
    access: &mut dyn StorageAccess,
    task: &WipeTask,
    state: &mut WipeState,
    stage: &Stage,
    frontend: &mut dyn WipeEventReceiver,
) -> Option<Rc<anyhow::Error>> {
    let mut stream = stage.stream(task.total_size, task.block_size, state.position);

    frontend.handle(task, state, WipeEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        let err_rc = Rc::from(err);
        frontend.handle(
            task,
            state,
            WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))),
        );
        return Some(Rc::clone(&err_rc));
    }

    while let Some(chunk) = stream.next() {
        if let Err(err) = access.write(chunk) {
            let err_rc = Rc::from(err);
            frontend.handle(
                task,
                state,
                WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))),
            );
            return Some(Rc::clone(&err_rc));
        }

        state.position += chunk.len() as u64;
        frontend.handle(task, state, WipeEvent::Progress(state.position));
    }

    if let Err(err) = access.flush() {
        let err_rc = Rc::from(err);
        frontend.handle(
            task,
            state,
            WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))),
        );
        return Some(Rc::clone(&err_rc));
    }

    frontend.handle(task, state, WipeEvent::StageCompleted(None));

    None
}

fn verify(
    access: &mut dyn StorageAccess,
    task: &WipeTask,
    state: &mut WipeState,
    stage: &Stage,
    frontend: &mut dyn WipeEventReceiver,
) -> Option<Rc<anyhow::Error>> {
    frontend.handle(task, state, WipeEvent::StageStarted);

    if let Err(err) = access.seek(state.position) {
        let err_rc = Rc::from(err);
        frontend.handle(
            task,
            state,
            WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))),
        );
        return Some(Rc::clone(&err_rc));
    }

    let mut stream = stage.stream(task.total_size, task.block_size, state.position);

    let buf = AlignedBuffer::new(task.block_size, task.block_size);

    while let Some(chunk) = stream.next() {
        let b = &mut buf.as_mut_slice()[..chunk.len()];

        if let Err(err) = access.read(b) {
            let err_rc = Rc::from(err);
            frontend.handle(
                task,
                state,
                WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))),
            );
            return Some(Rc::clone(&err_rc));
        }

        if b != chunk {
            let error = Rc::from(anyhow!("Verification failed!"));
            frontend.handle(
                task,
                state,
                WipeEvent::StageCompleted(Some(Rc::clone(&error))),
            );
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
    use anyhow::{Context, Result};
    use assert_matches::*;
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};
    use WipeEvent::*;

    #[test]
    fn test_wiping_happy_path() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
        );
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

        assert_eq!(
            storage.file.get_ref().iter().filter(|x| **x != 0u8).count(),
            0
        );
    }

    #[test]
    fn test_wiping_fill_failure() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_after_any(50000);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
        );
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, false);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 0);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StageCompleted(Some(_)))));
        assert_matches!(e.next(), Some((_, Completed(Some(_)))));

        assert_eq!(
            storage.file.get_ref().iter().filter(|x| **x != 0u8).count(),
            100000 - 32768
        );
    }

    #[test]
    fn test_wiping_validation_failure_with_retries() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_after_any(150000);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
        );
        let mut state = WipeState::default();
        state.retries_left = 8;
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
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 32768);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 32768);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));
    }

    #[test]
    fn test_wiping_write_failures_skips_bad_blocks() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_at(50000);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
        );
        let mut state = WipeState::default();
        state.retries_left = 8;
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
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification && s.position == 32768);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification && s.position == 32768);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));
    }

    #[test]
    fn test_wiping_validation_failure_without_retries() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_after_any(150000);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
        );
        let mut state = WipeState::default();
        state.retries_left = 0;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert_eq!(result, false);

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
        assert_matches!(e.next(), Some((_, Completed(Some(_)))));
    }

    struct StubReceiver {
        collected: Vec<(WipeState, WipeEvent)>,
    }

    impl StubReceiver {
        pub fn new() -> Self {
            StubReceiver {
                collected: Vec::new(),
            }
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
        failures: Vec<usize>,
        bad_blocks: Vec<u64>,
    }

    impl InMemoryStorage {
        fn new(size: usize) -> Self {
            InMemoryStorage {
                file: Cursor::new(vec![0xff; size]),
                size,
                total_written: 0,
                total_read: 0,
                failures: Vec::new(),
                bad_blocks: Vec::new(),
            }
        }

        fn fail_after_any(&mut self, amount: usize) -> () {
            self.failures.push(amount);
            self.failures.sort();
        }

        fn fail_at(&mut self, pos: u64) -> () {
            self.bad_blocks.push(pos);
            self.bad_blocks.sort();
        }

        fn check_for_traps(&mut self, read_bytes: usize, write_bytes: usize) -> Result<()> {
            let block_start = self.file.position();
            let block_end = block_start + write_bytes as u64;
            let is_bad_block = self
                .bad_blocks
                .iter()
                .find(|b| block_start <= **b && block_end > **b)
                .is_some();

            if is_bad_block {
                return Err(anyhow!("Mocked IO failure: bad block"));
            }

            let old_total = self.total_read + self.total_written;

            self.total_read += read_bytes;
            self.total_written += write_bytes;

            match self.failures.iter().find(|x| **x >= old_total) {
                Some(v) if old_total + read_bytes + write_bytes > *v => {
                    Err(anyhow!("Mocked IO failure"))
                }
                _ => Ok(()),
            }
        }
    }

    impl StorageAccess for InMemoryStorage {
        fn position(&mut self) -> Result<u64> {
            self.file.seek(SeekFrom::Current(0)).context("unexpected")
        }

        fn seek(&mut self, position: u64) -> Result<u64> {
            self.file
                .seek(SeekFrom::Start(position))
                .context("unexpected")
        }

        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            self.check_for_traps(buffer.len(), 0)?;
            self.file.read(buffer).context("unexpected")
        }

        fn write(&mut self, data: &[u8]) -> Result<()> {
            self.check_for_traps(0, data.len())?;
            self.file.write_all(data).context("unexpected")
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }
}
