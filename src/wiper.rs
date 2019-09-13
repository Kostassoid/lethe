use super::sanitization::*;
use super::storage::{StorageAccess, IoResult};
use std::io::{Error, ErrorKind};

pub enum Verify {
    No,
    Last,
    All
}

pub struct WiperTask {
    pub scheme: Scheme,
    pub verify: Verify,
    pub total_size: u64,
    pub block_size: usize
}

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

pub enum WiperEvent<'a> {
    FillStarted(&'a WiperTask, &'a WiperState),
    VerificationStarted(&'a WiperTask, &'a WiperState),
    Progress(u64),
    FillCompleted(IoResult<()>),
    VerificationCompleted(IoResult<()>),
    WipeAborted(&'a WiperTask, &'a WiperState),
    WipeCompleted(IoResult<()>)
}

pub trait WiperEventsReceiver {
    fn receive(&mut self, event: WiperEvent) -> ();
}

pub fn wipe(access: &mut StorageAccess, task: &WiperTask, state: &mut WiperState, events_receiver: &mut WiperEventsReceiver) -> IoResult<()> {

    let stages = &task.scheme.stages;
    for (i, stage) in stages.iter().enumerate() {

        let have_to_verify = match task.verify {
            Verify::No => false,
            Verify::Last if i + 1 == stages.len() => true,
            Verify::All => true,
            _ => false
        };

        loop {
            fill(access, task, state, events_receiver)?;

            if !have_to_verify {
                break;
            }

            if let Err(err) = verify(access, task, state, events_receiver) {
                eprintln!("Error: {}\nRetrying previous stage.", err);
            } else {
                break;
            }
        }

    }
    Ok(())
}

fn fill(access: &mut StorageAccess, task: &WiperTask, state: &mut WiperState, events_receiver: &mut WiperEventsReceiver) -> IoResult<()> {
    let stage = &task.scheme.stages[state.stage];

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    access.seek(state.position)?;

    events_receiver.receive(WiperEvent::FillStarted(task, state));

    while let Some(chunk) = stream.next() {
        access.write(chunk)?;

        state.position += chunk.len() as u64;
        events_receiver.receive(WiperEvent::Progress(state.position));
    };

    access.flush()?;
    events_receiver.receive(WiperEvent::FillCompleted(Ok(())));

    Ok(())
}

fn verify(access: &mut StorageAccess, task: &WiperTask, state: &mut WiperState, events_receiver: &mut WiperEventsReceiver) -> IoResult<()> {
    let stage = &task.scheme.stages[state.stage];

    access.seek(state.position)?;

    events_receiver.receive(WiperEvent::VerificationStarted(task, state));

    let mut stream = stage.stream(
        task.total_size, 
        task.block_size);

    let mut buf: Vec<u8> = vec![0; task.block_size];

    while let Some(chunk) = stream.next() {
        let b = &mut buf[..chunk.len()];
        access.read(b)?;
        if b != chunk {
            let error = Error::new(ErrorKind::InvalidData, "Verification failed!");
            events_receiver.receive(WiperEvent::VerificationCompleted(Err(Error::from(error.kind()))));
            return Err(Error::from(error.kind()));
        }

        state.position += chunk.len() as u64;
        events_receiver.receive(WiperEvent::Progress(state.position));
    }

    events_receiver.receive(WiperEvent::VerificationCompleted(Ok(())));

    Ok(())
}


