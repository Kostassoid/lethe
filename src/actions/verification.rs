use std::fmt::{self, Display, Formatter};
use std::ops::Range;

use anyhow::{Result, anyhow};

/// A percentage in the inclusive range `0..=100`.
#[derive(Debug, Clone)]
pub struct Percent(f32);

impl Percent {
    pub fn new(value: f32) -> Result<Percent> {
        if (0f32..=100f32).contains(&value) {
            Ok(Percent(value))
        } else {
            Err(anyhow!("Percent value {value} is outside of range 0..100"))
        }
    }

    /// Returns `value` scaled down to this percentage, rounded down.
    ///
    /// Multiplying before dividing keeps common round percentages exact in floating
    /// point; the result never exceeds `value`, and storage sizes are far below the
    /// range where `f64` loses integer precision.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn apply_to(&self, value: u64) -> u64 {
        (value as f64 * f64::from(self.0) / 100.0).floor() as u64
    }
}

impl Display for Percent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// When the written data should be verified, and how much of it.
#[derive(Debug, Clone)]
pub enum Verification {
    No,
    Last(Percent),
    All(Percent),
}

impl Display for Verification {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Verification::No => f.write_str("No"),
            Verification::Last(c) => write!(f, "Last stage only ({c}% coverage)"),
            Verification::All(c) => write!(f, "After each stage ({c}% coverage)"),
        }
    }
}

impl Verification {
    /// Returns the areas of the storage that a verification pass has to cover.
    pub fn coverage(&self, range: &Range<u64>, block_size: usize) -> Coverage {
        match self {
            Verification::No => Coverage::none(),
            Verification::Last(percent) | Verification::All(percent) => {
                Coverage::from_percent(range, block_size, percent)
            }
        }
    }
}

/// The areas of a storage device that a verification pass covers, as absolute byte ranges.
///
/// With partial coverage the beginning and the end of the wiped area are sampled, since
/// those regions are the most likely to reveal an incomplete wipe. Sampled areas are
/// rounded outward to whole blocks, as verification always operates on full blocks.
#[derive(Debug, Clone)]
pub struct Coverage {
    spans: Vec<Range<u64>>,
}

impl Coverage {
    fn none() -> Coverage {
        Coverage { spans: Vec::new() }
    }

    fn from_percent(range: &Range<u64>, block_size: usize, percent: &Percent) -> Coverage {
        let block = block_size as u64;
        let total = range.end - range.start;

        // Always sample at least the first and the last byte, even with 0% coverage.
        let covered = percent.apply_to(total).max(2);
        let half = covered / 2;

        if covered >= total {
            return Coverage {
                spans: vec![range.clone()],
            };
        }

        let prefix_end = ((range.start + half).div_ceil(block) * block).min(range.end);
        let suffix_start = (range.end - half) / block * block;

        let spans = if prefix_end >= suffix_start {
            // The sampled regions touch or overlap - verify everything.
            vec![range.clone()]
        } else {
            vec![range.start..prefix_end, suffix_start..range.end]
        };

        Coverage { spans }
    }

    /// Restricts the covered areas to `range`, dropping everything outside of it.
    ///
    /// Used when the wiped area is split into independently verified sections: every
    /// section keeps only the part of the globally sampled coverage it owns.
    pub fn clip(&self, range: &Range<u64>) -> Coverage {
        let spans = self
            .spans
            .iter()
            .filter_map(|span| {
                let start = span.start.max(range.start);
                let end = span.end.min(range.end);
                (start < end).then_some(start..end)
            })
            .collect();

        Coverage { spans }
    }

    pub fn spans(&self) -> &[Range<u64>] {
        &self.spans
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_percent_value_is_valid() {
        assert!(Percent::new(0f32).is_ok());
        assert!(Percent::new(0.001f32).is_ok());
        assert!(Percent::new(100f32).is_ok());
        assert!(Percent::new(67f32).is_ok());

        assert!(Percent::new(100.001f32).is_err());
        assert!(Percent::new(1000f32).is_err());
        assert!(Percent::new(-1f32).is_err());
    }

    #[test]
    fn test_percent_application_is_valid() {
        let p = Percent::new(23f32).unwrap();

        assert_eq!(25, p.apply_to(111));
        assert_eq!(0, p.apply_to(1));
    }

    #[test]
    fn test_full_coverage() {
        let range = 0..100_000u64;
        let c = Coverage::from_percent(&range, 8192, &Percent::new(100.0).unwrap());

        assert_eq!(c.spans(), &[range]);
    }

    #[test]
    fn test_partial_coverage_samples_both_ends() {
        let range = 0..100_000u64;
        let c = Coverage::from_percent(&range, 8192, &Percent::new(30.0).unwrap());

        // 30% of 100000 is 30000; 15000 bytes from each end, rounded outward to blocks.
        assert_eq!(c.spans(), &[0..16_384u64, 81_920u64..100_000u64]);
    }

    #[test]
    fn test_partial_coverage_with_offset_range() {
        let range = 65_536u64..1_000_000;
        let c = Coverage::from_percent(&range, 32768, &Percent::new(30.0).unwrap());

        // 30% of 934464 is 280339; ~140169 bytes from each end, rounded outward to blocks.
        assert_eq!(
            c.spans(),
            &[65_536u64..229_376u64, 851_968u64..1_000_000u64]
        );
    }

    #[test]
    fn test_zero_coverage_still_samples_first_and_last_blocks() {
        let range = 0..100_000u64;
        let c = Coverage::from_percent(&range, 8192, &Percent::new(0.0).unwrap());

        assert_eq!(c.spans(), &[0..8192u64, 98_304u64..100_000u64]);
    }

    #[test]
    fn test_sampling_regions_overlap_on_small_areas() {
        let range = 0..8192u64;
        let c = Coverage::from_percent(&range, 8192, &Percent::new(0.0).unwrap());

        assert_eq!(c.spans(), &[range]);
    }

    #[test]
    fn test_no_verification_has_no_coverage() {
        let v = Verification::No;
        assert_eq!(v.coverage(&(0..100u64), 512).spans().len(), 0);
    }

    #[test]
    fn test_clip_drops_spans_outside_the_range() {
        let coverage = Coverage::from_percent(&(0..100_000u64), 8192, &Percent::new(30.0).unwrap());

        assert_eq!(
            coverage.clip(&(0..40_960u64)).spans().first(),
            Some(&(0u64..16_384))
        );
        assert_eq!(coverage.clip(&(40_960u64..81_920u64)).spans().len(), 0);
        assert_eq!(
            coverage.clip(&(81_920u64..100_000u64)).spans().first(),
            Some(&(81_920u64..100_000))
        );
    }

    #[test]
    fn test_clip_trims_partial_overlap() {
        let coverage = Coverage::from_percent(&(0..100_000u64), 8192, &Percent::new(30.0).unwrap());

        assert_eq!(
            coverage.clip(&(8_192u64..90_112u64)).spans(),
            &[8_192..16_384u64, 81_920..90_112u64]
        );
    }
}
