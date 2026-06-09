use crate::actions::verification::*;
use crate::actions::{WipeEventHandler, WipeSession};
use crate::sanitization::scheme::Scheme;
use crate::storage::StorageAccess;
use anyhow::Result;
use std::ops::Range;

pub type StreamId = usize;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Step {
    Write(StreamId),
    Verify(StreamId),
}

#[derive(Clone, Debug)]
pub struct WipePlan {
    pub scheme: Scheme,
    pub steps: Vec<Step>,
    pub range: Range<u64>,
    pub block_size: usize,
    pub verification: Verification,
}

impl WipePlan {
    pub fn from(scheme: Scheme, verification: Verification, range: Range<u64>, block_size: usize) -> Result<WipePlan> {
        if range.end / block_size as u64 > 1 << 32 {
            Err(anyhow!(
                "Number of blocks in this device is more than 2^32. Try using a bigger block size."
            ))?;
        }
        if range.start >= range.end {
            Err(anyhow!("Starting offset is greater than the storage size"))?;
        }

        let corrected_offset = (range.start / block_size as u64) * block_size as u64;
        let range = corrected_offset..range.end;

        let mut steps = vec!();
        let total_stages = scheme.stages.len();

        for (i, s) in scheme.stages.iter().enumerate() {
            steps.push(Step::Write(i));

            match verification {
                Verification::No => {},
                Verification::Last(_) if i + 1 == total_stages => steps.push(Step::Verify(i)),
                Verification::All(_) => steps.push(Step::Verify(i)),
                _ => panic!("unknown verification {}", verification),
            };
        }

        Ok(WipePlan{ scheme, steps, range, block_size, verification })
    }

    pub fn execute(self, storage: Box<dyn StorageAccess>, event_handler: Box<dyn WipeEventHandler>, retries: u32) -> Result<()> {
        WipeSession::new(self, storage, event_handler, retries).run()
    }

    pub fn total_bytes(&self) -> u64 {
        self.range.end - self.range.start
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::sanitization::scheme::Stage;

    #[test]
    fn test_wipe_plan_validation() {
        let scheme = Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };

        assert!(WipePlan::from(scheme, Verification::No, 0..1<<40, 4).is_err());
    }
}
