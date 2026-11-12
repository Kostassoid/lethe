use std::collections::VecDeque;

use anyhow::{Result, anyhow};
use streaming_iterator::StreamingIterator;

use crate::actions::bad_blocks::BadBlocks;
use crate::actions::plan::{Step, StreamId, WipePlan};
use crate::sanitization::mem::AlignedBuffer;
use crate::sanitization::stream::Stream;
use crate::storage::{AsyncStorageAccess, IoKind, StorageAccess, StorageError};

type StepIndex = usize;
type BytePosition = u64;

/// Significant points in the lifetime of a wipe session, reported to a
/// [`WipeEventHandler`].
#[derive(Debug)]
pub enum WipeEvent {
    Created,
    Started,
    StepStarted(StepIndex),
    Progress(BytePosition),
    MarkedBlockAsBad(BytePosition),
    SkippedBadBlock(BytePosition),
    StepCompleted(StepIndex),
    StepFailed(StepIndex, String),
    Retrying(StepIndex, BytePosition),
    Completed,
    Failed(String),
}

/// Executes a [`WipePlan`] against a storage device, reporting progress through a
/// [`WipeEventHandler`].
pub struct WipeSession {
    event_handler: Box<dyn WipeEventHandler>,
    storage: Box<dyn StorageAccess>,
    plan: WipePlan,
    streams: Vec<Stream>,
    state: WipeSessionState,
}

/// Progress of a running session, exposed to event handlers.
pub struct WipeSessionState {
    pub position: u64,
    pub retries_left: u32,
    pub step: usize,
    pub bad_blocks: BadBlocks,
}

pub trait WipeEventHandler {
    fn handle(&mut self, plan: &WipePlan, state: &WipeSessionState, event: WipeEvent);
}

impl WipeSession {
    pub fn new(
        plan: WipePlan,
        storage: Box<dyn StorageAccess>,
        event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
    ) -> Self {
        let streams = plan
            .scheme
            .stages
            .iter()
            .map(|stage| Stream::new(stage, plan.range.clone(), plan.block_size))
            .collect();

        Self::with_streams(plan, storage, event_handler, retries, streams)
    }

    #[cfg(test)]
    pub(crate) fn new_with_seed(
        plan: WipePlan,
        storage: Box<dyn StorageAccess>,
        event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
        seed: [u8; crate::sanitization::stream::RANDOM_SEED_SIZE],
    ) -> Self {
        let streams = plan
            .scheme
            .stages
            .iter()
            .map(|stage| Stream::with_seed(stage, plan.range.clone(), plan.block_size, seed))
            .collect();

        Self::with_streams(plan, storage, event_handler, retries, streams)
    }

    fn with_streams(
        plan: WipePlan,
        storage: Box<dyn StorageAccess>,
        mut event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
        streams: Vec<Stream>,
    ) -> Self {
        let state = WipeSessionState {
            position: 0,
            retries_left: retries,
            step: 0,
            bad_blocks: BadBlocks::default(),
        };

        event_handler.handle(&plan, &state, WipeEvent::Created);

        WipeSession {
            event_handler,
            storage,
            plan,
            streams,
            state,
        }
    }

    pub(crate) fn run(mut self) -> Result<()> {
        self.publish(WipeEvent::Started);

        self.state.step = 0;
        self.state.position = self.plan.range.start;

        // When a verification attempt fails, the write step preceding it is re-executed
        // from the failure point; this is where the subsequent re-verification resumes.
        let mut resume_from: Option<u64> = None;

        let result = loop {
            if self.state.step >= self.plan.steps.len() {
                break Ok(());
            }

            let step = self.plan.steps[self.state.step];
            self.publish(WipeEvent::StepStarted(self.state.step));

            let result = match step {
                Step::Write(stream_id) => self.write_step(stream_id),
                Step::Verify(stream_id) => self.verify_step(stream_id),
            };

            match result {
                Ok(()) => {
                    self.publish(WipeEvent::StepCompleted(self.state.step));
                    self.state.step += 1;
                    self.state.position = resume_from.take().unwrap_or(self.plan.range.start);
                }
                Err(err) => {
                    self.publish(WipeEvent::StepFailed(self.state.step, err.to_string()));

                    if self.state.retries_left == 0 {
                        break Err(err.context(failure_context(step)));
                    }

                    self.state.retries_left -= 1;

                    if let Step::Verify(_) = step {
                        // Everything written since the verification failure point is no
                        // longer known to match the pattern: go back to the write step
                        // to re-write the remainder, then re-verify from the same point.
                        resume_from = Some(self.state.position);
                        self.state.step -= 1;
                    }

                    self.publish(WipeEvent::Retrying(self.state.step, self.state.position));
                }
            }
        };

        match &result {
            Ok(()) => self.publish(WipeEvent::Completed),
            Err(err) => self.publish(WipeEvent::Failed(err.to_string())),
        }

        result
    }

    fn write_step(&mut self, stream_id: StreamId) -> Result<()> {
        let mut io = SessionIO::new(
            &self.plan,
            &mut self.state,
            &mut *self.event_handler,
            &mut *self.storage,
        );
        let stream = &mut self.streams[stream_id];

        io.publish(WipeEvent::Progress(io.position()));
        io.skip_bad_blocks(self.plan.range.end)?;

        if io.at_end() {
            return Ok(());
        }

        stream.seek(io.position());

        let mut skip_next_chunk = false;
        while let Some(chunk) = stream.next() {
            if skip_next_chunk || !io.try_write(chunk)? {
                // The chunk's block is inaccessible: move past it and try to reposition
                // the storage for the next chunk.
                io.advance(chunk.len());
                skip_next_chunk = !io.try_seek()?;
            } else {
                io.advance(chunk.len());
            }
        }

        io.flush()
    }

    fn verify_step(&mut self, stream_id: StreamId) -> Result<()> {
        // A re-verification after a retry resumes from where the previous attempt
        // failed; everything before that point has already been checked.
        let resume_from = self.state.position;

        let mut io = SessionIO::new(
            &self.plan,
            &mut self.state,
            &mut *self.event_handler,
            &mut *self.storage,
        );
        let stream = &mut self.streams[stream_id];
        let mut scratch = AlignedBuffer::new(self.plan.block_size, self.plan.block_size);

        for span in self.plan.verification_coverage().spans() {
            let start = span.start.max(resume_from);
            if start >= span.end {
                continue; // nothing left to verify in this span
            }

            io.set_position(start);
            io.publish(WipeEvent::Progress(io.position()));
            io.skip_bad_blocks(span.end)?;

            if io.position() >= span.end {
                continue; // the whole span is inaccessible
            }

            stream.seek(io.position());

            while io.position() < span.end {
                let Some(chunk) = stream.next() else { break };

                if io.at_bad_block() {
                    io.publish(WipeEvent::SkippedBadBlock(io.position()));
                    io.advance(chunk.len());
                    io.try_seek()?;
                    continue;
                }

                io.verify_chunk(chunk, scratch.as_mut_slice())?;
                io.advance(chunk.len());
            }
        }

        Ok(())
    }

    fn publish(&mut self, event: WipeEvent) {
        self.event_handler.handle(&self.plan, &self.state, event);
    }
}

/// Executes a [`WipePlan`] with several positional I/O requests in flight.
///
/// The observable session semantics are the same as [`WipeSession`]: steps run in order,
/// bad blocks are marked and skipped, verification resumes from the failure point, and
/// progress only advances over contiguously completed chunks.
pub struct AsyncWipeSession {
    event_handler: Box<dyn WipeEventHandler>,
    storage: Box<dyn AsyncStorageAccess>,
    plan: WipePlan,
    streams: Vec<Stream>,
    state: WipeSessionState,
    depth: usize,
}

impl AsyncWipeSession {
    pub fn new(
        plan: WipePlan,
        storage: Box<dyn AsyncStorageAccess>,
        event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
        depth: usize,
    ) -> Self {
        let streams = plan
            .scheme
            .stages
            .iter()
            .map(|stage| Stream::new(stage, plan.range.clone(), plan.block_size))
            .collect();

        Self::with_streams(plan, storage, event_handler, retries, depth, streams)
    }

    #[cfg(test)]
    pub(crate) fn new_with_seed(
        plan: WipePlan,
        storage: Box<dyn AsyncStorageAccess>,
        event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
        depth: usize,
        seed: [u8; crate::sanitization::stream::RANDOM_SEED_SIZE],
    ) -> Self {
        let streams = plan
            .scheme
            .stages
            .iter()
            .map(|stage| Stream::with_seed(stage, plan.range.clone(), plan.block_size, seed))
            .collect();

        Self::with_streams(plan, storage, event_handler, retries, depth, streams)
    }

    fn with_streams(
        plan: WipePlan,
        storage: Box<dyn AsyncStorageAccess>,
        mut event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
        depth: usize,
        streams: Vec<Stream>,
    ) -> Self {
        let state = WipeSessionState {
            position: 0,
            retries_left: retries,
            step: 0,
            bad_blocks: BadBlocks::default(),
        };

        event_handler.handle(&plan, &state, WipeEvent::Created);

        AsyncWipeSession {
            event_handler,
            storage,
            plan,
            streams,
            state,
            depth: depth.max(1),
        }
    }

    pub(crate) fn run(mut self) -> Result<()> {
        self.publish(WipeEvent::Started);

        self.state.step = 0;
        self.state.position = self.plan.range.start;

        let mut resume_from: Option<u64> = None;

        let result = loop {
            if self.state.step >= self.plan.steps.len() {
                break Ok(());
            }

            let step = self.plan.steps[self.state.step];
            self.publish(WipeEvent::StepStarted(self.state.step));

            let result = match step {
                Step::Write(stream_id) => self.write_step(stream_id),
                Step::Verify(stream_id) => self.verify_step(stream_id),
            };

            match result {
                Ok(()) => {
                    self.publish(WipeEvent::StepCompleted(self.state.step));
                    self.state.step += 1;
                    self.state.position = resume_from.take().unwrap_or(self.plan.range.start);
                }
                Err(err) => {
                    self.publish(WipeEvent::StepFailed(self.state.step, err.to_string()));

                    if self.state.retries_left == 0 {
                        break Err(err.context(failure_context(step)));
                    }

                    self.state.retries_left -= 1;

                    if let Step::Verify(_) = step {
                        resume_from = Some(self.state.position);
                        self.state.step -= 1;
                    }

                    self.publish(WipeEvent::Retrying(self.state.step, self.state.position));
                }
            }
        };

        match &result {
            Ok(()) => self.publish(WipeEvent::Completed),
            Err(err) => self.publish(WipeEvent::Failed(err.to_string())),
        }

        result
    }

    fn write_step(&mut self, stream_id: StreamId) -> Result<()> {
        let start = self.state.position;
        let mut io = AsyncSessionIO::new(
            &self.plan,
            &mut self.state,
            &mut *self.event_handler,
            &mut *self.storage,
            self.depth,
            false,
        );
        let stream = &mut self.streams[stream_id];

        io.begin(start);
        io.skip_bad_blocks(self.plan.range.end);

        if io.at_end() {
            return io.finish(true);
        }

        stream.seek(io.submitted);

        while let Some(chunk) = stream.next() {
            io.write_chunk(chunk)?;
        }

        io.finish(true)
    }

    fn verify_step(&mut self, stream_id: StreamId) -> Result<()> {
        let resume_from = self.state.position;

        for span in self.plan.verification_coverage().spans() {
            let start = span.start.max(resume_from);
            if start >= span.end {
                continue;
            }

            let mut io = AsyncSessionIO::new(
                &self.plan,
                &mut self.state,
                &mut *self.event_handler,
                &mut *self.storage,
                self.depth,
                true,
            );
            let stream = &mut self.streams[stream_id];

            io.begin(start);
            io.skip_bad_blocks(span.end);

            if io.submitted >= span.end {
                io.finish(false)?;
                continue;
            }

            stream.seek(io.submitted);

            while io.submitted < span.end {
                let Some(chunk) = stream.next() else { break };
                io.read_chunk(chunk)?;
            }

            io.finish(false)?;
        }

        Ok(())
    }

    fn publish(&mut self, event: WipeEvent) {
        self.event_handler.handle(&self.plan, &self.state, event);
    }
}

#[derive(Clone, Copy)]
struct SlotMeta {
    position: u64,
    len: usize,
}

/// How many times a short transfer is retried before it is treated as a failure.
const MAX_SHORT_TRANSFER_RETRIES: u8 = 4;

struct QueueEntry {
    slot: Option<usize>,
    len: usize,
}

/// The pipelined I/O face of an [`AsyncWipeSession`], mirroring [`SessionIO`] but
/// submitting multiple requests before reaping their completions.
struct AsyncSessionIO<'a> {
    plan: &'a WipePlan,
    state: &'a mut WipeSessionState,
    event_handler: &'a mut dyn WipeEventHandler,
    storage: &'a mut dyn AsyncStorageAccess,
    buffers: Vec<AlignedBuffer>,
    expected: Vec<Vec<u8>>,
    slots: Vec<Option<SlotMeta>>,
    short_retries: Vec<u8>,
    free_slots: Vec<usize>,
    queue: VecDeque<QueueEntry>,
    /// Absolute position of the next chunk to submit.
    submitted: u64,
    /// Absolute position up to which every chunk has completed.
    progress: u64,
    inflight: usize,
    verifying: bool,
}

impl<'a> AsyncSessionIO<'a> {
    fn new(
        plan: &'a WipePlan,
        state: &'a mut WipeSessionState,
        event_handler: &'a mut dyn WipeEventHandler,
        storage: &'a mut dyn AsyncStorageAccess,
        depth: usize,
        verifying: bool,
    ) -> Self {
        let depth = depth.max(1).min(storage.depth().max(1));
        let block_size = plan.block_size;

        let buffers = (0..depth)
            .map(|_| AlignedBuffer::new(block_size, block_size))
            .collect();
        let expected = if verifying {
            (0..depth).map(|_| vec![0u8; block_size]).collect()
        } else {
            Vec::new()
        };
        let slots = (0..depth).map(|_| None).collect();
        let short_retries = vec![0; depth];
        let free_slots = (0..depth).rev().collect();

        AsyncSessionIO {
            plan,
            state,
            event_handler,
            storage,
            buffers,
            expected,
            slots,
            short_retries,
            free_slots,
            queue: VecDeque::new(),
            submitted: 0,
            progress: 0,
            inflight: 0,
            verifying,
        }
    }

    fn publish(&mut self, event: WipeEvent) {
        self.event_handler.handle(self.plan, self.state, event);
    }

    fn begin(&mut self, position: u64) {
        self.submitted = position;
        self.progress = position;
        self.state.position = position;
        self.publish(WipeEvent::Progress(position));
    }

    fn at_end(&self) -> bool {
        self.submitted >= self.plan.range.end
    }

    fn at_bad_block(&self, position: u64) -> bool {
        self.state
            .bad_blocks
            .contains(self.plan.block_index(position))
    }

    fn skip_bad_blocks(&mut self, limit: u64) {
        while self.submitted < limit {
            if !self.at_bad_block(self.submitted) {
                return;
            }

            let len = usize::try_from((limit - self.submitted).min(self.plan.block_size as u64))
                .expect("block size fits usize");
            self.publish(WipeEvent::SkippedBadBlock(self.submitted));
            self.queue.push_back(QueueEntry { slot: None, len });
            self.submitted += len as u64;
            self.drain_completed();
        }
    }

    /// Submits the next write chunk, reaping a completion first if all slots are busy.
    fn write_chunk(&mut self, chunk: &[u8]) -> Result<()> {
        let position = self.submitted;
        let len = chunk.len();

        if self.at_bad_block(position) {
            self.queue.push_back(QueueEntry { slot: None, len });
            self.submitted += len as u64;
            self.drain_completed();
            return Ok(());
        }

        self.ensure_free_slot()?;

        let slot = self.free_slots.pop().expect("a slot is available");
        self.buffers[slot].as_mut_slice()[..len].copy_from_slice(chunk);

        let buffer = &mut self.buffers[slot].as_mut_slice()[..len];
        self.storage.submit(slot, position, buffer, IoKind::Write)?;
        self.track(slot, position, len);
        Ok(())
    }

    /// Submits the next read chunk and keeps the expected pattern for comparison.
    fn read_chunk(&mut self, chunk: &[u8]) -> Result<()> {
        let position = self.submitted;
        let len = chunk.len();

        if self.at_bad_block(position) {
            self.queue.push_back(QueueEntry { slot: None, len });
            self.submitted += len as u64;
            self.drain_completed();
            return Ok(());
        }

        self.ensure_free_slot()?;

        let slot = self.free_slots.pop().expect("a slot is available");
        self.expected[slot][..len].copy_from_slice(chunk);

        let buffer = &mut self.buffers[slot].as_mut_slice()[..len];
        self.storage.submit(slot, position, buffer, IoKind::Read)?;
        self.track(slot, position, len);
        Ok(())
    }

    fn track(&mut self, slot: usize, position: u64, len: usize) {
        self.slots[slot] = Some(SlotMeta { position, len });
        self.queue.push_back(QueueEntry {
            slot: Some(slot),
            len,
        });
        self.submitted += len as u64;
        self.inflight += 1;
    }

    fn ensure_free_slot(&mut self) -> Result<()> {
        while self.inflight >= self.buffers.len() {
            self.reap_one()?;
        }

        Ok(())
    }

    fn reap_one(&mut self) -> Result<()> {
        let completion = self.storage.reap()?;
        let slot = completion.slot;
        let meta = *self.slots[slot]
            .as_ref()
            .expect("completion references an in-flight slot");

        if let Ok(bytes) = &completion.result
            && *bytes < meta.len
            && self.short_retries[slot] < MAX_SHORT_TRANSFER_RETRIES
        {
            // Windows (and some devices) may report a short transfer instead of an
            // error. Re-issue the request, mirroring read_exact/write_all in the
            // synchronous implementation.
            self.short_retries[slot] += 1;
            let kind = if self.verifying {
                IoKind::Read
            } else {
                IoKind::Write
            };
            let buffer = &mut self.buffers[slot].as_mut_slice()[..meta.len];
            self.storage.submit(slot, meta.position, buffer, kind)?;
            return Ok(());
        }

        self.slots[slot] = None;
        self.short_retries[slot] = 0;
        self.inflight -= 1;
        self.free_slots.push(slot);

        if let Some(entry) = self.queue.iter_mut().find(|entry| entry.slot == Some(slot)) {
            entry.slot = None;
        }

        match completion.result {
            Ok(bytes) => {
                if bytes != meta.len {
                    let operation = if self.verifying { "read" } else { "write" };

                    return self.fail(
                        meta.position,
                        anyhow!(
                            "Unexpected {operation} transfer length {bytes} (expected {}) at position {} (wiped area ends at {})",
                            meta.len,
                            meta.position,
                            self.plan.range.end
                        ),
                    );
                }

                if self.verifying {
                    let actual = &self.buffers[slot].as_slice()[..meta.len];
                    let expected = &self.expected[slot][..meta.len];

                    if actual != expected {
                        return self.fail(
                            meta.position,
                            anyhow!("Verification failed at position {}", meta.position),
                        );
                    }
                }
            }
            Err(err) => {
                if let Some(StorageError::BadBlock) = underlying_storage_error(&err) {
                    self.state.position = meta.position;
                    self.state
                        .bad_blocks
                        .mark(self.plan.block_index(meta.position));
                    self.publish(WipeEvent::MarkedBlockAsBad(meta.position));
                } else {
                    return self.fail(meta.position, err);
                }
            }
        }

        self.drain_completed();
        Ok(())
    }

    /// Drains the remaining requests, then reports the earliest failed position so that
    /// a retry resumes from the same place the sequential implementation would use.
    fn fail(&mut self, position: u64, error: anyhow::Error) -> Result<()> {
        let mut earliest = position;

        while self.inflight > 0 {
            let Ok(completion) = self.storage.reap() else {
                break;
            };

            let slot = completion.slot;
            let Some(meta) = self.slots[slot].take() else {
                continue;
            };

            self.inflight -= 1;
            self.free_slots.push(slot);
            match completion.result {
                Err(_) => earliest = earliest.min(meta.position),
                Ok(bytes) => {
                    let short = bytes != meta.len;
                    let mismatch = self.verifying
                        && !short
                        && self.buffers[slot].as_slice()[..meta.len]
                            != self.expected[slot][..meta.len];

                    if short || mismatch {
                        earliest = earliest.min(meta.position);
                    }
                }
            }
        }

        self.storage.drain().ok();
        self.state.position = earliest;

        if self.verifying {
            Err(anyhow!("Verification failed at position {earliest}"))
        } else {
            Err(error)
        }
    }

    fn drain_completed(&mut self) {
        while self.queue.front().is_some_and(|entry| entry.slot.is_none()) {
            let entry = self.queue.pop_front().expect("queue is not empty");
            self.progress += entry.len as u64;
            self.state.position = self.progress;
            self.publish(WipeEvent::Progress(self.progress));
        }
    }

    fn finish(&mut self, flush: bool) -> Result<()> {
        while self.inflight > 0 {
            self.reap_one()?;
        }

        self.drain_completed();

        if flush {
            self.storage.flush()?;
        }

        Ok(())
    }
}

impl Drop for AsyncSessionIO<'_> {
    fn drop(&mut self) {
        if self.inflight == 0 {
            return;
        }

        // Never leave requests pointing into this session's buffers after an early
        // return. If the backend cannot drain, leak the buffers rather than risk the
        // kernel writing into freed memory.
        if self.storage.drain().is_err() {
            for buffer in std::mem::take(&mut self.buffers) {
                std::mem::forget(buffer);
            }
        }
    }
}

fn failure_context(step: Step) -> &'static str {
    match step {
        Step::Write(_) => "Failed to fill stream",
        Step::Verify(_) => "Failed to verify stream",
    }
}

/// The I/O face of a wipe session: couples the storage device with the session state
/// and progress reporting, and encapsulates the bad-block handling policy shared by
/// writing and verification.
///
/// Kept separate from [`WipeSession`] so that the pattern streams can be borrowed
/// independently while a step is running.
struct SessionIO<'a> {
    plan: &'a WipePlan,
    state: &'a mut WipeSessionState,
    event_handler: &'a mut dyn WipeEventHandler,
    storage: &'a mut dyn StorageAccess,
}

impl<'a> SessionIO<'a> {
    fn new(
        plan: &'a WipePlan,
        state: &'a mut WipeSessionState,
        event_handler: &'a mut dyn WipeEventHandler,
        storage: &'a mut dyn StorageAccess,
    ) -> Self {
        SessionIO {
            plan,
            state,
            event_handler,
            storage,
        }
    }

    fn publish(&mut self, event: WipeEvent) {
        self.event_handler.handle(self.plan, self.state, event);
    }

    fn position(&self) -> u64 {
        self.state.position
    }

    fn set_position(&mut self, position: u64) {
        self.state.position = position.min(self.plan.range.end);
    }

    fn at_end(&self) -> bool {
        self.state.position >= self.plan.range.end
    }

    /// Moves the cursor forward, clamping to the end of the planned area.
    fn advance(&mut self, bytes: usize) {
        self.state.position = (self.state.position + bytes as u64).min(self.plan.range.end);
        self.publish(WipeEvent::Progress(self.state.position));
    }

    fn at_bad_block(&self) -> bool {
        self.state.bad_blocks.contains(self.current_block())
    }

    fn current_block(&self) -> u32 {
        self.plan.block_index(self.state.position)
    }

    fn mark_current_block_bad(&mut self) {
        self.state.bad_blocks.mark(self.current_block());
        self.publish(WipeEvent::MarkedBlockAsBad(self.state.position));
    }

    /// Advances the cursor past bad blocks until the storage is positioned at a healthy
    /// block, or until `limit` is reached.
    fn skip_bad_blocks(&mut self, limit: u64) -> Result<()> {
        while self.state.position < limit {
            if self.at_bad_block() || !self.try_seek()? {
                self.publish(WipeEvent::SkippedBadBlock(self.state.position));
                self.advance(self.plan.block_size);
            } else {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Seeks the storage to the current cursor position.
    ///
    /// Returns `false` if the block is (or has just been marked) bad; any other failure
    /// is an error.
    fn try_seek(&mut self) -> Result<bool> {
        if self.at_bad_block() {
            return Ok(false);
        }

        if let Err(err) = self.storage.seek(self.state.position) {
            return match underlying_storage_error(&err) {
                Some(StorageError::BadBlock) => {
                    self.mark_current_block_bad();
                    Ok(false)
                }
                _ => Err(err),
            };
        }

        Ok(true)
    }

    /// Writes a chunk at the current cursor position.
    ///
    /// Returns `false` if the block is (or has just been marked) bad; any other failure
    /// is an error.
    fn try_write(&mut self, chunk: &[u8]) -> Result<bool> {
        if self.at_bad_block() {
            return Ok(false);
        }

        if let Err(err) = self.storage.write(chunk) {
            return match underlying_storage_error(&err) {
                Some(StorageError::BadBlock) => {
                    self.mark_current_block_bad();
                    Ok(false)
                }
                _ => Err(err),
            };
        }

        Ok(true)
    }

    /// Reads a chunk at the current cursor position and compares it against the
    /// expected pattern.
    fn verify_chunk(&mut self, expected: &[u8], scratch: &mut [u8]) -> Result<()> {
        let actual = &mut scratch[..expected.len()];
        self.read_exact(actual)?;

        if actual != expected {
            return Err(anyhow!(
                "Verification failed at position {}",
                self.state.position
            ));
        }

        Ok(())
    }

    /// Fills `buffer` from the storage, tolerating short reads.
    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<()> {
        let mut filled = 0;
        while filled < buffer.len() {
            let read = self.storage.read(&mut buffer[filled..])?;
            if read == 0 {
                return Err(anyhow!(
                    "Unexpected end of storage at position {}",
                    self.state.position + filled as u64
                ));
            }
            filled += read;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.storage.flush()
    }
}

// taken directly from https://docs.rs/anyhow/1.0.9/anyhow/struct.Error.html#example
fn underlying_storage_error(error: &anyhow::Error) -> Option<&StorageError> {
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
    use crate::actions::verification::{Percent, Verification};
    use crate::sanitization::stream::RANDOM_SEED_SIZE;
    use crate::sanitization::{Scheme, SchemeRepo, Stage};
    use crate::storage::IoCompletion;
    use WipeEvent::*;
    use anyhow::Result;
    use assert_matches::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_wipe_task_validation() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();

        assert!(WipePlan::new(scheme.clone(), Verification::No, 0..1 << 32, 1).is_ok());
        assert!(WipePlan::new(scheme.clone(), Verification::No, 0..1 << 35, 8).is_ok());
        assert!(WipePlan::new(scheme.clone(), Verification::No, 0..1 << 33, 1).is_err());
        assert!(WipePlan::new(scheme.clone(), Verification::No, 0..1 << 36, 8).is_err());
    }

    #[test]
    fn test_wiping_from_beginning() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 0);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_from_offset() {
        let scheme = Scheme {
            description: "Single zeroes fill x2".to_string(),
            stages: vec![Stage::zero(), Stage::zero()],
        };
        let storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::All(Percent::new(100.0).unwrap()),
            70000..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 0);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(StepStarted(2)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(2)));
        assert_matches!(e.next(), Some(StepStarted(3)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(3)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_fill_failure() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("zero").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_after_any(50000);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 0);

        assert!(result.is_err());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(StepFailed(0, _)));
        assert_matches!(e.next(), Some(Failed(_)));
    }

    #[test]
    fn test_wiping_validation_failure_with_retries() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_after_any(150_000);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 8);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(StepFailed(1, _)));
        assert_matches!(e.next(), Some(Retrying(0, 32768)));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_write_failures_skips_bad_blocks() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_at(50000);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 8);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(SkippedBadBlock(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_skip_bad_blocks_at_beginning() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_at(0);
        storage.fail_at(32768);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 8);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(SkippedBadBlock(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(SkippedBadBlock(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_skip_bad_blocks_at_ending() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_at(99999);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 8);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(SkippedBadBlock(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_handle_completely_corrupt_storage() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_at(0);
        storage.fail_at(32768);
        storage.fail_at(65536);
        storage.fail_at(98304);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 8);

        assert!(result.is_ok());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();
        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(MarkedBlockAsBad(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(SkippedBadBlock(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(SkippedBadBlock(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(SkippedBadBlock(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(SkippedBadBlock(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_wiping_validation_failure_without_retries() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let mut storage = InMemoryStorage::new(100_000);
        let block_size = 32768;
        let receiver = StubReceiver::new();

        storage.fail_after_any(150_000);

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(Percent::new(100.0).unwrap()),
            0..storage.size as u64,
            block_size,
        )
        .unwrap();
        let result = plan.execute(Box::new(storage), Box::new(receiver.clone()), 0);

        assert!(result.is_err());

        let collected = receiver.collected.borrow();
        let mut e = collected.iter();

        assert_matches!(e.next(), Some(Created));
        assert_matches!(e.next(), Some(Started));
        assert_matches!(e.next(), Some(StepStarted(0)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(Progress(65536)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(32768)));
        assert_matches!(e.next(), Some(StepFailed(1, _)));
        assert_matches!(e.next(), Some(Failed(_)));
    }

    #[test]
    fn test_wiping_validation_with_partial_coverage() {
        let schemes = SchemeRepo::default();
        let scheme = schemes.find("random").unwrap();
        let storage = InMemoryStorage::new(100_000);
        let block_size = 8192;
        let receiver = StubReceiver::new();

        let plan = WipePlan::new(
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
        assert_matches!(e.next(), Some(Created));
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
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(0)));
        // Verification covers the first two and the last three blocks only.
        assert_matches!(e.next(), Some(StepStarted(1)));
        assert_matches!(e.next(), Some(Progress(0)));
        assert_matches!(e.next(), Some(Progress(8192)));
        assert_matches!(e.next(), Some(Progress(16384)));
        assert_matches!(e.next(), Some(Progress(81920)));
        assert_matches!(e.next(), Some(Progress(90112)));
        assert_matches!(e.next(), Some(Progress(98304)));
        assert_matches!(e.next(), Some(Progress(100_000)));
        assert_matches!(e.next(), Some(StepCompleted(1)));
        assert_matches!(e.next(), Some(Completed));
    }

    #[test]
    fn test_random_wipe_with_bad_blocks_uses_correct_stream_positions() {
        let stage = Stage::random();
        let scheme = Scheme {
            description: "random".to_string(),
            stages: vec![stage.clone()],
        };
        let block_size = 8192;
        let size = 100_000;
        let seed = [0x5a; RANDOM_SEED_SIZE];
        let (mut storage, data) = InMemoryStorage::shared(size);

        // Two consecutive bad blocks at the very start (exercising the initial
        // skip and re-seek), one in the verified prefix/suffix and one in the
        // unverified gap.
        storage.fail_at(0);
        storage.fail_at(8192);
        storage.fail_at(49152);
        storage.fail_at(90112);

        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(30.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();

        let reference = fill_reference(&stage, plan.range.clone(), block_size, seed);
        let receiver = StubReceiver::new();

        let result =
            WipeSession::new_with_seed(plan, Box::new(storage), Box::new(receiver), 8, seed).run();
        assert!(result.is_ok());

        let data = data.lock().expect("test storage lock poisoned");
        for start in (0..size).step_by(block_size) {
            let end = (start + block_size).min(size);
            let actual = &data[start..end];

            if matches!(start, 0 | 8192 | 49152 | 90112) {
                assert!(
                    actual.iter().all(|byte| *byte == 0xff),
                    "bad block at {start} was modified"
                );
            } else {
                assert_eq!(
                    actual,
                    &reference[start..end],
                    "block at {start} does not match the reference stream"
                );
            }
        }
    }

    #[test]
    fn test_random_wipe_retry_rewrites_correct_stream_positions() {
        let stage = Stage::random();
        let scheme = Scheme {
            description: "random".to_string(),
            stages: vec![stage.clone()],
        };
        let block_size = 8192;
        let size = 100_000;
        let seed = [0x33; RANDOM_SEED_SIZE];
        let (mut storage, data) = InMemoryStorage::shared(size);

        // Cross this threshold during verification of the fourth chunk, forcing
        // a verification failure followed by a rewrite from that position.
        storage.fail_after_any(size + 3 * block_size + 1);

        let plan = WipePlan::new(
            scheme,
            Verification::All(Percent::new(100.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();

        let reference = fill_reference(&stage, plan.range.clone(), block_size, seed);
        let receiver = StubReceiver::new();

        let result = WipeSession::new_with_seed(
            plan,
            Box::new(storage),
            Box::new(receiver.clone()),
            8,
            seed,
        )
        .run();
        assert!(result.is_ok());

        assert_eq!(
            &data.lock().expect("test storage lock poisoned")[..],
            reference.as_slice()
        );

        let collected = receiver.collected.borrow();
        assert!(
            collected
                .iter()
                .any(|event| matches!(event, WipeEvent::Retrying(0, 24_576))),
            "expected the verification to fail and resume the write at 24576"
        );
    }

    #[test]
    fn test_async_wiping_and_verification() {
        let scheme = SchemeRepo::default().find("zero").unwrap().clone();
        let size = 100_000;
        let block_size = 8192;
        let data = Arc::new(Mutex::new(vec![0xffu8; size]));
        let storage = MockAsyncStorage::new(Arc::clone(&data), 4);
        let receiver = StubReceiver::new();

        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(100.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();

        let result = plan.execute_async(Box::new(storage), Box::new(receiver.clone()), 0, 4);
        assert!(result.is_ok());
        assert!(
            data.lock()
                .expect("test storage lock poisoned")
                .iter()
                .all(|byte| *byte == 0),
            "the async wipe did not cover the whole area"
        );

        let collected = receiver.collected.borrow();
        assert!(
            collected.iter().any(|event| matches!(event, Completed)),
            "the async session did not complete"
        );
    }

    #[test]
    fn test_async_retries_short_transfers() {
        let scheme = SchemeRepo::default().find("zero").unwrap().clone();
        let size = 100_000;
        let block_size = 8192;
        let data = Arc::new(Mutex::new(vec![0xffu8; size]));
        let mut storage = MockAsyncStorage::new(Arc::clone(&data), 4);
        storage.zero_transfers = vec![24_576];
        let receiver = StubReceiver::new();

        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(100.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();

        let result = plan.execute_async(Box::new(storage), Box::new(receiver), 0, 4);
        assert!(result.is_ok());
        assert!(
            data.lock()
                .expect("test storage lock poisoned")
                .iter()
                .all(|byte| *byte == 0),
            "the retried transfer did not cover the whole area"
        );
    }

    #[test]
    fn test_async_random_wipe_skips_bad_blocks_and_keeps_stream_positions() {
        let stage = Stage::random();
        let scheme = Scheme {
            description: "random".to_string(),
            stages: vec![stage.clone()],
        };
        let block_size = 8192;
        let size = 100_000;
        let seed = [0x5a; RANDOM_SEED_SIZE];
        let data = Arc::new(Mutex::new(vec![0xffu8; size]));
        let mut storage = MockAsyncStorage::new(Arc::clone(&data), 8);
        storage.bad_blocks = vec![0, 8192, 49152];

        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(30.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();
        let reference = fill_reference(&stage, plan.range.clone(), block_size, seed);
        let receiver = StubReceiver::new();

        let result = AsyncWipeSession::new_with_seed(
            plan,
            Box::new(storage),
            Box::new(receiver),
            8,
            8,
            seed,
        )
        .run();
        assert!(result.is_ok());

        let data = data.lock().expect("test storage lock poisoned");
        for start in (0..size).step_by(block_size) {
            let end = (start + block_size).min(size);

            if matches!(start, 0 | 8192 | 49152) {
                assert!(
                    data[start..end].iter().all(|byte| *byte == 0xff),
                    "bad block at {start} was modified"
                );
            } else {
                assert_eq!(
                    &data[start..end],
                    &reference[start..end],
                    "block at {start} does not match the reference stream"
                );
            }
        }
    }

    #[test]
    fn test_async_verification_failure_rewrites_from_the_failing_position() {
        let stage = Stage::random();
        let scheme = Scheme {
            description: "random".to_string(),
            stages: vec![stage.clone()],
        };
        let block_size = 8192;
        let size = 100_000;
        let seed = [0x33; RANDOM_SEED_SIZE];
        let data = Arc::new(Mutex::new(vec![0xffu8; size]));
        let mut storage = MockAsyncStorage::new(Arc::clone(&data), 8);
        storage.corrupt_reads = vec![24_576];

        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(100.0).unwrap()),
            0..size as u64,
            block_size,
        )
        .unwrap();
        let reference = fill_reference(&stage, plan.range.clone(), block_size, seed);
        let receiver = StubReceiver::new();

        let result = AsyncWipeSession::new_with_seed(
            plan,
            Box::new(storage),
            Box::new(receiver.clone()),
            8,
            8,
            seed,
        )
        .run();
        assert!(result.is_ok());

        assert_eq!(
            &data.lock().expect("test storage lock poisoned")[..],
            reference.as_slice()
        );

        let collected = receiver.collected.borrow();
        assert!(
            collected
                .iter()
                .any(|event| matches!(event, WipeEvent::Retrying(0, 24_576))),
            "expected the async verification to resume the write at 24576"
        );
    }

    fn fill_reference(
        stage: &Stage,
        range: std::ops::Range<u64>,
        block_size: usize,
        seed: [u8; RANDOM_SEED_SIZE],
    ) -> Vec<u8> {
        let size = usize::try_from(range.end - range.start).expect("test area is addressable");
        let mut reference = vec![0u8; size];
        let mut stream = Stream::with_seed(stage, range, block_size, seed);
        let mut position = 0;

        while let Some(chunk) = stream.next() {
            reference[position..position + chunk.len()].copy_from_slice(chunk);
            position += chunk.len();
        }

        assert_eq!(position, size);
        reference
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
        fn handle(&mut self, _plan: &WipePlan, _state: &WipeSessionState, event: WipeEvent) {
            self.collected.borrow_mut().push(event);
        }
    }

    struct InMemoryStorage {
        data: Arc<Mutex<Vec<u8>>>,
        position: u64,
        size: usize,
        total_written: usize,
        total_read: usize,
        failures: Vec<usize>,
        bad_blocks: Vec<u64>,
    }

    impl InMemoryStorage {
        fn new(size: usize) -> Self {
            Self::shared(size).0
        }

        /// Returns the storage and a handle to its underlying bytes, so tests can
        /// inspect the final device contents after a session consumes the storage.
        fn shared(size: usize) -> (Self, Arc<Mutex<Vec<u8>>>) {
            let data = Arc::new(Mutex::new(vec![0xff; size]));

            let storage = InMemoryStorage {
                data: Arc::clone(&data),
                position: 0,
                size,
                total_written: 0,
                total_read: 0,
                failures: Vec::new(),
                bad_blocks: Vec::new(),
            };

            (storage, data)
        }

        fn fail_after_any(&mut self, amount: usize) {
            self.failures.push(amount);
            self.failures.sort_unstable();
        }

        fn fail_at(&mut self, pos: u64) {
            self.bad_blocks.push(pos);
            self.bad_blocks.sort_unstable();
        }

        fn byte_position(&self) -> usize {
            usize::try_from(self.position).expect("position does not fit into memory")
        }

        fn check_for_traps(&mut self, read_bytes: usize, write_bytes: usize) -> Result<()> {
            let block_start = self.position;
            let block_end = block_start + write_bytes as u64;
            let is_bad_block = self
                .bad_blocks
                .iter()
                .any(|b| block_start <= *b && block_end > *b);

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
        fn seek(&mut self, position: u64) -> Result<u64> {
            self.position = position;
            Ok(position)
        }

        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            self.check_for_traps(buffer.len(), 0)?;

            let start = self.byte_position();
            let data = self.data.lock().expect("test storage lock poisoned");
            if start >= data.len() {
                return Ok(0);
            }

            let count = (data.len() - start).min(buffer.len());
            buffer[..count].copy_from_slice(&data[start..start + count]);
            self.position += count as u64;
            Ok(count)
        }

        fn write(&mut self, data: &[u8]) -> Result<()> {
            self.check_for_traps(0, data.len())?;

            let start = self.byte_position();
            let end = start + data.len();
            let mut target = self.data.lock().expect("test storage lock poisoned");
            if target.len() < end {
                target.resize(end, 0);
            }
            target[start..end].copy_from_slice(data);
            self.position = end as u64;
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Completes every request immediately but defers reporting through `reap`, so the
    /// session exercises its multi-request bookkeeping.
    struct MockAsyncStorage {
        data: Arc<Mutex<Vec<u8>>>,
        completions: VecDeque<IoCompletion>,
        bad_blocks: Vec<u64>,
        corrupt_reads: Vec<u64>,
        zero_transfers: Vec<u64>,
        depth: usize,
    }

    impl MockAsyncStorage {
        fn new(data: Arc<Mutex<Vec<u8>>>, depth: usize) -> Self {
            MockAsyncStorage {
                data,
                completions: VecDeque::new(),
                bad_blocks: Vec::new(),
                corrupt_reads: Vec::new(),
                zero_transfers: Vec::new(),
                depth,
            }
        }
    }

    impl AsyncStorageAccess for MockAsyncStorage {
        fn depth(&self) -> usize {
            self.depth
        }

        fn submit(
            &mut self,
            slot: usize,
            position: u64,
            buffer: &mut [u8],
            kind: IoKind,
        ) -> Result<()> {
            let start = usize::try_from(position).expect("test position fits usize");
            let end = start + buffer.len();

            let result = if self
                .bad_blocks
                .iter()
                .any(|block| start as u64 <= *block && end as u64 > *block)
            {
                Err(StorageError::BadBlock.into())
            } else if self.zero_transfers.contains(&position) {
                self.zero_transfers.retain(|value| *value != position);
                Ok(0)
            } else {
                let mut data = self.data.lock().expect("test storage lock poisoned");
                if start >= data.len() {
                    Ok(0)
                } else {
                    let count = (data.len() - start).min(buffer.len());
                    match kind {
                        IoKind::Read => {
                            buffer[..count].copy_from_slice(&data[start..start + count]);
                            if self.corrupt_reads.contains(&position) {
                                buffer[0] ^= 0xff;
                                self.corrupt_reads.retain(|value| *value != position);
                            }
                        }
                        IoKind::Write => {
                            data[start..start + count].copy_from_slice(&buffer[..count]);
                        }
                    }
                    Ok(count)
                }
            };

            self.completions.push_back(IoCompletion { slot, result });
            Ok(())
        }

        fn reap(&mut self) -> Result<IoCompletion> {
            self.completions
                .pop_front()
                .ok_or_else(|| anyhow!("No pending mock completion"))
        }

        fn drain(&mut self) -> Result<()> {
            self.completions.clear();
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }
}
