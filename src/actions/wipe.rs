use crate::actions::marker::{BlockMarker, RoaringBlockMarker};
use crate::sanitization::mem::*;
use crate::sanitization::*;
use crate::storage::{StorageAccess, StorageError};
use anyhow::Result;
use std::cell::RefCell;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

#[derive(Debug)]
pub enum Verify {
    No,
    Last,
    All,
}

impl Display for Verify {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Verify::No => f.write_str("No"),
            Verify::Last => f.write_str("Last stage only"),
            Verify::All => f.write_str("After each stage"),
        }
    }
}

#[derive(Debug)]
pub struct WipeTask {
    pub scheme: Scheme,
    pub verify: Verify,
    pub total_size: u64,
    pub block_size: usize,
    pub offset: u64,
}

#[derive(Debug, Clone)]
pub struct WipeState {
    pub stage: usize,
    pub at_verification: bool,
    pub position: u64,
    pub retries_left: u32,
    pub bad_blocks: Rc<RefCell<dyn BlockMarker>>,
}

pub struct WipeRun<'a> {
    pub access: &'a mut dyn StorageAccess,
    pub task: &'a WipeTask,
    pub state: &'a mut WipeState,
    pub frontend: &'a mut dyn WipeEventReceiver,
}

impl Default for WipeState {
    fn default() -> Self {
        WipeState {
            stage: 0,
            at_verification: false,
            position: 0,
            retries_left: 0,
            bad_blocks: Rc::new(RefCell::new(RoaringBlockMarker::new())),
        }
    }
}

impl WipeTask {
    pub fn new(
        scheme: Scheme,
        verify: Verify,
        total_size: u64,
        block_size: usize,
        offset: u64,
    ) -> Result<Self> {
        if total_size / block_size as u64 > 1 << 32 {
            Err(anyhow!(
                "Number of blocks in this device is more than 2^32. Try using a bigger block size."
            ))?;
        }
        if offset >= total_size {
            Err(anyhow!("Starting offset is greater than the storage size"))?;
        }
        let corrected_offset = (offset / block_size as u64) * block_size as u64;

        Ok(WipeTask {
            scheme,
            verify,
            total_size,
            block_size,
            offset: corrected_offset,
        })
    }
}

#[derive(Debug)]
pub enum WipeEvent {
    Created,
    Started,
    StageStarted,
    Progress(u64),
    MarkedBlockAsBad(u64),
    StageCompleted(Option<Rc<anyhow::Error>>),
    Retrying,
    Completed(Option<Rc<anyhow::Error>>),
    Fatal(anyhow::Error),
}

pub trait WipeEventReceiver {
    fn handle(&mut self, task: &WipeTask, state: &WipeState, event: WipeEvent) -> ();
}

impl WipeTask {
    pub fn run(
        &self,
        access: &mut dyn StorageAccess,
        state: &mut WipeState,
        frontend: &mut dyn WipeEventReceiver,
    ) -> bool {
        WipeRun {
            access,
            task: &self,
            state,
            frontend,
        }
        .run()
    }
}

impl WipeRun<'_> {
    fn publish(&mut self, event: WipeEvent) {
        self.frontend.handle(self.task, self.state, event)
    }

    fn build_stream(&self, stage: &Stage) -> SanitizationStream {
        stage.stream(
            self.task.total_size,
            self.task.block_size,
            self.state.position,
        )
    }

    fn advance(&mut self, bytes: usize) {
        self.state.position += bytes as u64;
        if self.state.position > self.task.total_size {
            self.state.position = self.task.total_size
        }
        self.publish(WipeEvent::Progress(self.state.position));
    }

    fn at_the_end(&self) -> bool {
        self.state.position >= self.task.total_size
    }

    fn current_block_number(&self) -> u32 {
        (self.state.position / self.task.block_size as u64) as u32
    }

    fn is_at_bad_block(&self) -> bool {
        self.state
            .bad_blocks
            .borrow()
            .is_marked(self.current_block_number())
    }

    fn mark_bad_block(&mut self) -> () {
        self.state
            .bad_blocks
            .borrow_mut()
            .mark(self.current_block_number());
        self.publish(WipeEvent::MarkedBlockAsBad(self.state.position));
    }

    fn try_seek(&mut self) -> Result<bool> {
        if self.is_at_bad_block() {
            return Ok(false);
        }

        if let Err(err) = self.access.seek(self.state.position) {
            return match underlying_storage_error(&err) {
                Some(StorageError::BadBlock) => {
                    self.mark_bad_block();
                    Ok(false)
                }
                _ => Err(err),
            };
        }

        Ok(true)
    }

    fn try_write(&mut self, chunk: &[u8]) -> Result<bool> {
        if self.is_at_bad_block() {
            return Ok(false);
        }

        if let Err(err) = self.access.write(chunk) {
            return match underlying_storage_error(&err) {
                Some(StorageError::BadBlock) => {
                    self.mark_bad_block();
                    Ok(false)
                }
                _ => Err(err),
            };
        }
        Ok(true)
    }

    fn seek_to_the_next_safe_position(&mut self) -> Result<()> {
        loop {
            if self.at_the_end() {
                break;
            }

            if self.is_at_bad_block() || !self.try_seek()? {
                self.advance(self.task.block_size);
                continue;
            }

            break;
        }
        Ok(())
    }

    fn run(&mut self) -> bool {
        self.publish(WipeEvent::Started);

        let stages = &self.task.scheme.stages;

        let mut wipe_error = None;

        for (i, stage) in stages.iter().enumerate() {
            let have_to_verify = match self.task.verify {
                Verify::No => false,
                Verify::Last if i + 1 == stages.len() => true,
                Verify::All => true,
                _ => false,
            };

            self.state.stage = i;
            self.state.position = self.task.offset;
            self.state.at_verification = false;

            let stage_error = loop {
                let watermark = self.state.position;

                self.publish(WipeEvent::StageStarted);
                if let Err(err) = self.fill(stage) {
                    let err_rc = Rc::from(err);
                    self.publish(WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));

                    if self.state.retries_left > 0 {
                        self.state.retries_left -= 1;
                        self.publish(WipeEvent::Retrying);
                        continue;
                    }

                    break Some(err_rc);
                }
                self.publish(WipeEvent::StageCompleted(None));

                if !have_to_verify {
                    break None;
                }

                self.state.position = watermark;
                self.state.at_verification = true;

                self.publish(WipeEvent::StageStarted);
                if let Err(err) = self.verify(stage) {
                    let err_rc = Rc::from(err);
                    self.publish(WipeEvent::StageCompleted(Some(Rc::clone(&err_rc))));

                    if self.state.retries_left > 0 {
                        self.state.retries_left -= 1;
                        self.state.at_verification = false;
                        self.publish(WipeEvent::Retrying);
                        continue;
                    }
                    break Some(err_rc);
                }
                self.publish(WipeEvent::StageCompleted(None));
                break None;
            };

            if stage_error.is_some() {
                wipe_error = stage_error;
                break;
            };
        }

        let result = wipe_error.is_none();
        self.publish(WipeEvent::Completed(wipe_error));

        result
    }

    fn fill(&mut self, stage: &Stage) -> Result<()> {
        self.publish(WipeEvent::Progress(self.state.position));

        self.seek_to_the_next_safe_position()?;

        if self.at_the_end() {
            return Ok(());
        }

        let mut stream = self.build_stream(stage);
        let mut skip_next = false;

        while let Some(chunk) = stream.next() {
            if skip_next || !self.try_write(chunk)? {
                self.advance(chunk.len());
                skip_next = !self.try_seek()?;
                continue;
            }

            self.advance(chunk.len());
        }

        self.access.flush()?;

        Ok(())
    }

    fn verify(&mut self, stage: &Stage) -> Result<()> {
        self.publish(WipeEvent::Progress(self.state.position));

        self.seek_to_the_next_safe_position()?;

        if self.at_the_end() {
            return Ok(());
        }

        let mut stream = self.build_stream(stage);

        let buf = AlignedBuffer::new(self.task.block_size, self.task.block_size);

        while let Some(chunk) = stream.next() {
            if self.is_at_bad_block() {
                self.advance(chunk.len());
                self.try_seek()?;
                continue;
            }

            let b = &mut buf.as_mut_slice()[..chunk.len()];

            self.access.read(b)?;

            if b != chunk {
                Err(anyhow!("Verification failed!"))?;
            }

            self.advance(chunk.len());
        }

        Ok(())
    }
}

// taken directly from https://docs.rs/anyhow/1.0.9/anyhow/struct.Error.html#example
pub fn underlying_storage_error(error: &anyhow::Error) -> Option<&StorageError> {
    for cause in error.chain() {
        if let Some(storage_error) = cause.downcast_ref::<StorageError>() {
            return Some(storage_error);
        }
    }
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
    fn test_wipe_task_validation() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();

        assert!(WipeTask::new(scheme.clone(), Verify::No, 1 << 32, 1, 0).is_ok());
        assert!(WipeTask::new(scheme.clone(), Verify::No, 1 << 35, 8, 0).is_ok());
        assert!(WipeTask::new(scheme.clone(), Verify::No, 1 << 33, 1, 0).is_err());
        assert!(WipeTask::new(scheme.clone(), Verify::No, 1 << 36, 8, 0).is_err());
    }

    #[test]
    fn test_wiping_from_beginning() {
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
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
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
    fn test_wiping_from_offset() {
        let scheme = Scheme {
            description: "Single zeroes fill x2".to_string(),
            stages: vec![Stage::zero(), Stage::zero()],
        };
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        let task = WipeTask::new(
            scheme.clone(),
            Verify::All,
            storage.size as u64,
            block_size,
            70000,
        )
        .unwrap();
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));

        assert_eq!(
            storage
                .file
                .get_ref()
                .iter()
                .skip(task.offset as usize)
                .filter(|x| **x != 0u8)
                .count(),
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
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(!result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
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
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 8;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StageCompleted(Some(_)))));
        assert_matches!(e.next(), Some((_, Retrying)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(32768))));
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
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 8;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));
    }

    #[test]
    fn test_wiping_skip_bad_blocks_at_beginning() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_at(0);
        storage.fail_at(32768);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 8;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));
    }

    #[test]
    fn test_wiping_skip_bad_blocks_at_ending() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_at(99999);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 8;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((_, Completed(None))));
    }

    #[test]
    fn test_wiping_handle_completely_corrupt_storage() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        storage.fail_at(0);
        storage.fail_at(32768);
        storage.fail_at(65536);
        storage.fail_at(98304);

        let task = WipeTask::new(
            scheme.clone(),
            Verify::Last,
            storage.size as u64,
            block_size,
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 8;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
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
            0,
        )
        .unwrap();
        let mut state = WipeState::default();
        state.retries_left = 0;
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(!result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StageCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StageStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
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
            println!("{:?}", event);
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
                return Err(StorageError::BadBlock.into());
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
