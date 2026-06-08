use crate::actions::marker::{BlockMarker, RoaringBlockMarker};
use crate::actions::plan::WipePlan;
use crate::actions::verification::*;
use crate::actions::{Step, StreamId};
use crate::sanitization::mem::*;
use crate::sanitization::stream::Stream;
use crate::storage::{StorageAccess, StorageError};
use anyhow::Result;
use streaming_iterator::StreamingIterator;

pub struct WipeSession {
    pub event_handler: Box<dyn WipeEventHandler>,
    pub storage: Box<dyn StorageAccess>,
    pub plan: WipePlan,
    pub streams: Vec<Stream>,
    pub state: WipeSessionState,
}

type StepIndex = usize;
type BytePosition = u64;

#[derive(Debug)]
pub enum WipeEvent {
    Created,
    Started,
    StepStarted(StepIndex),
    Progress(BytePosition),
    SkippedTo(BytePosition),
    MarkedBlockAsBad(BytePosition),
    StepCompleted(StepIndex),
    StepFailed(StepIndex, String),
    Retrying(StepIndex, BytePosition),
    Completed,
    Failed(String),
    Fatal(anyhow::Error),
}

pub struct WipeSessionState {
    pub position: u64,
    pub retries_left: u32,
    pub step: usize,
    pub bad_blocks: Box<dyn BlockMarker>,
    pub coverage: CoverageFilter,
}

pub trait WipeEventHandler {
    fn handle(&mut self, state: &WipeSessionState, event: WipeEvent) -> ();
}

impl WipeSession {
    pub fn new(
        plan: WipePlan,
        storage_access: Box<dyn StorageAccess>,
        mut event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
    ) -> Self {
        let coverage = plan.verification.build_map(plan.total_bytes()); // TODO: use blocks

        let state = WipeSessionState {
            position: 0,
            retries_left: retries,
            step: 0,
            bad_blocks: Box::new(RoaringBlockMarker::new()),
            coverage,
        };

        let streams = plan.scheme.stages.iter()
            .map(|s| Stream::from(s, plan.range.end, plan.block_size))
            .collect();

        event_handler.handle(&state, WipeEvent::Created);

        WipeSession {
            event_handler,
            storage: storage_access,
            plan,
            streams,
            state,
        }
    }

    fn publish(&mut self, event: WipeEvent) {
        self.event_handler.handle(&self.state, event)
    }

    fn advance(&mut self, bytes: usize) {
        self.state.position += bytes as u64;
        if self.state.position > self.plan.range.end {
            self.state.position = self.plan.range.end
        }
        self.publish(WipeEvent::Progress(self.state.position));
    }

    fn at_the_end(&self) -> bool {
        self.state.position >= self.plan.range.end
    }

    fn current_block_number(&self) -> u32 {
        (self.state.position / self.plan.block_size as u64) as u32
    }

    fn is_at_bad_block(&self) -> bool {
        self.state.bad_blocks.is_marked(self.current_block_number())
    }

    fn mark_bad_block(&mut self) -> () {
        self.state.bad_blocks.mark(self.current_block_number());
        self.publish(WipeEvent::MarkedBlockAsBad(self.state.position));
    }

    fn try_seek(&mut self) -> Result<bool> {
        if self.is_at_bad_block() {
            return Ok(false);
        }

        if let Err(err) = self.storage.seek(self.state.position) {
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

        if let Err(err) = self.storage.write(chunk) {
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
                self.advance(self.plan.block_size);
                continue;
            }

            break;
        }
        Ok(())
    }

    pub(crate) fn run(mut self) -> Result<()> {
        self.publish(WipeEvent::Started);

        self.state.step = 0;
        self.state.position = self.plan.range.start;

        let result = loop {
            if self.state.step >= self.plan.steps.len() {
                break Ok(());
            }

            let step = self.plan.steps[self.state.step].clone();

            self.publish(WipeEvent::StepStarted(self.state.step));

            match step {
                Step::Write(stream_id) => {
                    let stream = &mut self.streams[stream_id];
                    stream.seek(self.state.position);

                    if let Err(err) = self.fill(stream) {
                        let err_message = err.to_string();
                        self.publish(WipeEvent::StepFailed(self.state.step, err_message));

                        if self.state.retries_left > 0 {
                            self.state.retries_left -= 1;
                            self.publish(WipeEvent::Retrying(self.state.step, self.state.position));
                            continue;
                        }

                        break Err(err.context("Failed to fill stream"));
                    }

                    self.state.step += 1;
                    self.state.position = self.plan.range.start;
                }
                Step::Verify(stream_id) => {
                    let stream = &mut self.streams[stream_id];
                    stream.seek(self.state.position);
                    if let Err(err) = self.verify(stream) {
                        let err_message = err.to_string();
                        self.publish(WipeEvent::StepFailed(self.state.step, err_message));

                        if self.state.retries_left > 0 {
                            self.state.retries_left -= 1;
                            self.state.step -= 1;
                            self.publish(WipeEvent::Retrying(self.state.step, self.state.position));
                            continue;
                        }

                        break Err(err.context("Failed to verify stream"));
                    }

                    self.state.step += 1;
                    self.state.position = self.plan.range.start;
                }
            }

            self.publish(WipeEvent::StepCompleted(self.state.step));
        };

        match &result {
            Ok(_) =>self.publish(WipeEvent::Completed),
            Err(err) =>self.publish(WipeEvent::Failed(err.to_string())),
        }

        result
    }

    fn fill(&mut self, stream: &mut Stream) -> Result<()> {
        self.publish(WipeEvent::Progress(stream.position));

        self.seek_to_the_next_safe_position()?;

        if self.at_the_end() {
            return Ok(());
        }

        let mut skip_next = false;

        while let Some(chunk) = stream.next() {
            if skip_next || !self.try_write(chunk)? {
                self.advance(chunk.len());
                skip_next = !self.try_seek()?;
                continue;
            }

            self.advance(chunk.len());
        }

        self.storage.flush()?;

        Ok(())
    }

    fn verify(&mut self, stream: &mut Stream) -> Result<()> {
        self.publish(WipeEvent::Progress(stream.position));

        self.seek_to_the_next_safe_position()?;

        if self.at_the_end() {
            return Ok(());
        }

        let buf = AlignedBuffer::new(self.plan.block_size, self.plan.block_size);

        while let Some(chunk) = stream.next() {
            if self.is_at_bad_block() {
                self.advance(chunk.len());
                self.try_seek()?;
                continue;
            }

            // if !self.coverage.in_range(self.position) {
            //     let next_position = self.coverage.next();
            // }

            let b = &mut buf.as_mut_slice()[..chunk.len()];

            self.storage.read(b)?;

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
    use crate::sanitization::{Scheme, SchemeRepo, Stage};
    use anyhow::{Context, Result};
    use assert_matches::*;
    use std::cell::RefCell;
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};
    use WipeEvent::*;

    #[test]
    fn test_wipe_task_validation() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();

        assert!(WipePlan::from(scheme.clone(), Verification::No, 0..1 << 32, 1).is_ok());
        assert!(WipePlan::from(scheme.clone(), Verification::No, 0..1 << 35, 8).is_ok());
        assert!(WipePlan::from(scheme.clone(), Verification::No, 0..1 << 33, 1).is_err());
        assert!(WipePlan::from(scheme.clone(), Verification::No, 0..1 << 36, 8).is_err());
    }

    #[test]
    fn test_wiping_from_beginning() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100000);
        let block_size = 32768;
        let mut receiver = StubReceiver::new();

        let plan = WipePlan::from(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver), 0);

        assert!(result.is_ok());

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::All(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            70000,
            0,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            0,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(!result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StepCompleted(Some(_)))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            8,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StepCompleted(Some(_)))));
        assert_matches!(e.next(), Some((_, Retrying)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            8,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            8,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            8,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            8,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, MarkedBlockAsBad(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
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
            Verification::Last(Percent::new(100.0).unwrap()),
            storage.size as u64,
            block_size,
            0,
            0,
        )
        .unwrap();
        let mut state = (&task).into();
        let result = task.run(&mut storage, &mut state, &mut receiver);

        assert!(!result);

        let mut e = receiver.collected.iter();
        assert_matches!(e.next(), Some((_, Started)));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if !s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, Progress(65536))));
        assert_matches!(e.next(), Some((_, Progress(98304))));
        assert_matches!(e.next(), Some((_, Progress(100000))));
        assert_matches!(e.next(), Some((_, StepCompleted(None))));
        assert_matches!(e.next(), Some((ref s, StepStarted)) if s.at_verification);
        assert_matches!(e.next(), Some((_, Progress(0))));
        assert_matches!(e.next(), Some((_, Progress(32768))));
        assert_matches!(e.next(), Some((_, StepFailed(0, _))));
        assert_matches!(e.next(), Some((_, Failed(_))));
    }

    #[test]
    fn test_wiping_validation_with_partial_coverage() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let storage = InMemoryStorage::new(100000);
        let block_size = 8192;
        let receiver = StubReceiver::new();

        let plan = WipePlan::from(
            scheme.clone(),
            Verification::Last(Percent::new(30.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();

        let result = plan
            .clone()
            .execute(Box::new(storage), Box::new(receiver.clone()), 0);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(8192)));
        assert_matches!(e.next(), Some(Progress(16384)));
        assert_matches!(e.next(), Some(Progress(24576)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(40960)));
        assert_matches!(e.next(), Some(Progress(49152)));
        assert_matches!(e.next(), Some(Progress(57344)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(73728)));
        assert_matches!(e.next(), Some(Progress(81920)));
        assert_matches!(e.next(), Some(Progress(90112)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[derive(Clone)]
    struct StubReceiver {
        collected: Rc<RefCell<Vec<WipeEvent>>>,
    }

    impl StubReceiver {
        pub fn new() -> Self {
            StubReceiver {
                collected: Rc::new(RefCell::new(vec![])),
            }
        }
    }

    impl WipeEventHandler for StubReceiver {
        fn handle(&mut self, _state: &WipeSessionState, event: WipeEvent) -> () {
            println!("{:?}", event);
            self.collected.borrow_mut().push(event);
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
