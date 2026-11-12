use std::ops::Range;
use std::thread;

use anyhow::{Result, anyhow};

use crate::actions::verification::{Coverage, Verification};
use crate::actions::wipe::{WipeEventHandler, WipeSession};
use crate::sanitization::scheme::Scheme;
use crate::storage::StorageAccess;

/// A handler factory for one parallel section. The handler is built inside the section's
/// thread, so it does not need to be `Send`.
pub type SectionHandlerFactory = Box<dyn FnOnce() -> Box<dyn WipeEventHandler> + Send>;

pub type StreamId = usize;

/// A single step of a wipe plan: either write a stage's pattern or verify it.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Step {
    Write(StreamId),
    Verify(StreamId),
}

#[derive(Clone, Debug)]
pub struct WipePlan {
    pub scheme: Scheme,
    pub steps: Vec<Step>,
    /// Area of the storage to wipe, in absolute byte positions. The start is aligned
    /// down to a block boundary, as all I/O operates on whole blocks.
    pub range: Range<u64>,
    pub block_size: usize,
    pub verification: Verification,
    /// Coverage precomputed for the whole original plan, clipped to this plan's range.
    /// Set on sections produced by [`WipePlan::split`].
    coverage_override: Option<Coverage>,
}

impl WipePlan {
    pub fn new(
        scheme: Scheme,
        verification: Verification,
        range: Range<u64>,
        block_size: usize,
    ) -> Result<WipePlan> {
        if scheme.stages.is_empty() {
            Err(anyhow!("The scheme must contain at least one stage"))?;
        }
        if block_size == 0 || !block_size.is_power_of_two() {
            Err(anyhow!("Block size must be a power of two"))?;
        }
        if range.end.div_ceil(block_size as u64) > 1 << 32 {
            // Block numbers are tracked as u32; the highest possible block number
            // (and thus the block count) must fit the range.
            Err(anyhow!(
                "Number of blocks in this device is more than 2^32. Try using a bigger block size."
            ))?;
        }
        if range.start >= range.end {
            Err(anyhow!("Starting offset is greater than the storage size"))?;
        }

        let aligned_start = range.start / block_size as u64 * block_size as u64;
        let range = aligned_start..range.end;

        let mut steps = Vec::new();
        let total_stages = scheme.stages.len();

        for i in 0..total_stages {
            steps.push(Step::Write(i));

            let is_last_stage = i + 1 == total_stages;
            let verify_this_stage = match verification {
                Verification::No => false,
                Verification::All(_) => true,
                Verification::Last(_) => is_last_stage,
            };

            if verify_this_stage {
                steps.push(Step::Verify(i));
            }
        }

        Ok(WipePlan {
            scheme,
            steps,
            range,
            block_size,
            verification,
            coverage_override: None,
        })
    }

    pub fn execute(
        self,
        storage: Box<dyn StorageAccess>,
        event_handler: Box<dyn WipeEventHandler>,
        retries: u32,
    ) -> Result<()> {
        WipeSession::new(self, storage, event_handler, retries).run()
    }

    /// Splits the planned area into up to `parts` contiguous, block-aligned sections.
    ///
    /// Every section keeps the globally computed verification coverage, clipped to its
    /// own range, so partial coverage still samples the beginning and the end of the
    /// whole device rather than of each section.
    pub fn split(&self, parts: usize) -> Vec<WipePlan> {
        let parts = parts.max(1);
        let total_blocks = self.total_blocks();

        if parts == 1 || total_blocks <= 1 {
            return vec![self.clone()];
        }

        let parts = u64::try_from(parts).map_or(total_blocks, |value| value.min(total_blocks));
        let blocks_per_section = total_blocks.div_ceil(parts);
        let coverage = self.verification_coverage();
        let block_size = self.block_size as u64;

        let mut sections = Vec::new();
        let mut start_block = 0u64;

        while start_block < total_blocks {
            let end_block = (start_block + blocks_per_section).min(total_blocks);

            let start = self.range.start + start_block * block_size;
            let end = (self.range.start + end_block * block_size).min(self.range.end);
            let range = start..end;

            let mut section = self.clone();
            section.range = range.clone();
            section.coverage_override = Some(coverage.clip(&range));

            sections.push(section);
            start_block = end_block;
        }

        sections
    }

    /// Runs the given sections concurrently, each against its own storage handle.
    ///
    /// The sections are usually produced by [`WipePlan::split`]; this is the parallel
    /// counterpart of [`WipePlan::execute`].
    pub fn execute_sections(
        sections: Vec<WipePlan>,
        storages: Vec<Box<dyn StorageAccess>>,
        handlers: Vec<SectionHandlerFactory>,
        retries: u32,
    ) -> Result<()> {
        if sections.len() != storages.len() || sections.len() != handlers.len() {
            return Err(anyhow!(
                "Mismatched section, storage and event handler counts"
            ));
        }

        let results = thread::scope(|scope| {
            let mut threads = Vec::with_capacity(sections.len());

            for ((section, storage), handler) in sections.into_iter().zip(storages).zip(handlers) {
                threads.push(
                    scope.spawn(move || {
                        WipeSession::new(section, storage, handler(), retries).run()
                    }),
                );
            }

            threads
                .into_iter()
                .map(|thread| {
                    thread
                        .join()
                        .unwrap_or_else(|_| Err(anyhow!("A wipe section thread panicked")))
                })
                .collect::<Vec<_>>()
        });

        let mut error = None;

        for result in results {
            if let Err(err) = result
                && error.is_none()
            {
                error = Some(err);
            }
        }

        error.map_or(Ok(()), Err)
    }

    pub fn total_bytes(&self) -> u64 {
        self.range.end - self.range.start
    }

    /// Total number of blocks covered by the plan, including a possibly shorter final one.
    pub fn total_blocks(&self) -> u64 {
        self.total_bytes().div_ceil(self.block_size as u64)
    }

    /// Number of the block containing the given absolute byte position.
    pub fn block_index(&self, position: u64) -> u32 {
        let index = position / self.block_size as u64;
        u32::try_from(index).expect("block index out of u32 range")
    }

    /// The areas a verification pass of this plan has to cover.
    pub fn verification_coverage(&self) -> Coverage {
        self.coverage_override
            .clone()
            .unwrap_or_else(|| self.verification.coverage(&self.range, self.block_size))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::actions::verification::Percent;
    use crate::sanitization::scheme::Stage;

    #[test]
    fn test_wipe_plan_validation() {
        let scheme = || Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };

        assert!(WipePlan::new(scheme(), Verification::No, 0..1 << 40, 4).is_err());
        assert!(WipePlan::new(scheme(), Verification::No, 0..10, 0).is_err());
        assert!(WipePlan::new(scheme(), Verification::No, 0..10, 3).is_err());
        assert!(WipePlan::new(scheme(), Verification::No, 10..10, 512).is_err());

        let (start, end) = (10u64, 5u64); // start after end
        assert!(WipePlan::new(scheme(), Verification::No, start..end, 512).is_err());

        assert!(
            WipePlan::new(
                Scheme {
                    description: "empty".to_string(),
                    stages: vec![]
                },
                Verification::No,
                0..10,
                512
            )
            .is_err()
        );
    }

    #[test]
    fn test_block_count_limit_is_exact() {
        let scheme = || Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };

        // Exactly 2^32 blocks still fit, one more byte does not.
        assert!(WipePlan::new(scheme(), Verification::No, 0..(1u64 << 32) * 8, 8).is_ok());
        assert!(WipePlan::new(scheme(), Verification::No, 0..(1u64 << 32) * 8 + 1, 8).is_err());
    }

    #[test]
    fn test_offset_is_aligned_down_to_block_boundary() {
        let scheme = Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };

        let plan = WipePlan::new(scheme, Verification::No, 70_000..100_000, 32_768).unwrap();

        assert_eq!(plan.range, 65_536..100_000);
    }

    #[test]
    fn test_split_covers_area_and_clips_global_coverage() {
        let scheme = Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };
        let plan = WipePlan::new(
            scheme,
            Verification::Last(Percent::new(30.0).unwrap()),
            0..100_000,
            8192,
        )
        .unwrap();

        let sections = plan.split(3);

        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].range, 0..40_960);
        assert_eq!(sections[1].range, 40_960..81_920);
        assert_eq!(sections[2].range, 81_920..100_000);

        assert_eq!(
            sections[0].verification_coverage().spans().first(),
            Some(&(0u64..16_384))
        );
        assert_eq!(sections[1].verification_coverage().spans().len(), 0);
        assert_eq!(
            sections[2].verification_coverage().spans().first(),
            Some(&(81_920u64..100_000))
        );

        let blocks: u64 = sections.iter().map(WipePlan::total_blocks).sum();
        assert_eq!(blocks, plan.total_blocks());
    }

    #[test]
    fn test_split_is_capped_by_the_number_of_blocks() {
        let scheme = Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero()],
        };
        let plan = WipePlan::new(scheme, Verification::No, 0..8192, 4096).unwrap();

        assert_eq!(plan.split(10).len(), 2);
    }

    #[test]
    fn test_steps_depend_on_verification_mode() {
        let scheme = Scheme {
            description: "test".to_string(),
            stages: vec![Stage::zero(), Stage::zero(), Stage::zero()],
        };
        let percent = crate::actions::verification::Percent::new(50.0).unwrap();

        let plan = WipePlan::new(scheme.clone(), Verification::No, 0..4096, 512).unwrap();
        assert_eq!(
            plan.steps,
            vec![Step::Write(0), Step::Write(1), Step::Write(2)]
        );

        let plan = WipePlan::new(
            scheme.clone(),
            Verification::Last(percent.clone()),
            0..4096,
            512,
        )
        .unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::Write(0),
                Step::Write(1),
                Step::Write(2),
                Step::Verify(2)
            ]
        );

        let plan = WipePlan::new(scheme, Verification::All(percent), 0..4096, 512).unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::Write(0),
                Step::Verify(0),
                Step::Write(1),
                Step::Verify(1),
                Step::Write(2),
                Step::Verify(2)
            ]
        );
    }
}
