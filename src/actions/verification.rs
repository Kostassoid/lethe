use anyhow::Result;
use std::cmp::max;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone)]
pub struct Percent {
    v: f32,
}

impl Percent {
    pub fn new(value: f32) -> Result<Percent> {
        if !(0f32..=100f32).contains(&value) {
            Err(anyhow!(
                "Percent value {} is outside of range 0..100",
                value
            ))
        } else {
            Ok(Percent { v: value })
        }
    }

    pub fn apply_to(&self, value: u64) -> u64 {
        ((value as f32) * (self.v * 0.01)).floor() as u64
    }
}

#[derive(Debug, Clone)]
pub enum Verification {
    No,
    Last(Percent),
    All(Percent),
}

impl Display for Verification {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Verification::No => f.write_str("No"),
            Verification::Last(c) => f.write_str(&format!("Last stage only ({}% coverage)", c.v)),
            Verification::All(c) => f.write_str(&format!("After each stage ({}% coverage)", c.v)),
        }
    }
}

impl Verification {
    pub fn build_map(&self, total: u64) -> CoverageFilter {
        match self {
            Verification::No => CoverageFilter::empty(),
            Verification::Last(c) => CoverageFilter::from(total, c),
            Verification::All(c) => CoverageFilter::from(total, c),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverageFilter {
    low: u64,
    high: u64,
    max: u64,
}

impl CoverageFilter {
    pub fn empty() -> CoverageFilter {
        CoverageFilter {
            low: 0,
            high: 0,
            max: 0,
        }
    }

    fn from(total: u64, ratio: &Percent) -> CoverageFilter {
        let coverage_total = max(ratio.apply_to(total), 2);

        CoverageFilter {
            low: coverage_total / 2,
            high: (total - coverage_total / 2),
            max: total,
        }
    }

    fn should_verify(&self, at: u64) -> bool {
        at < self.low || (at >= self.high && at < self.max)
    }

    fn next_verifiable(&self, starting_at: u64) -> Option<u64> {
        if self.should_verify(starting_at) {
            return Some(starting_at);
        }

        Some(self.high).filter(|h| starting_at < *h)
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
    fn test_coverage_distribution_from_percents() {
        let c = CoverageFilter::from(8192, &Percent::new(20f32).unwrap());
        assert_eq!(819, c.low);
        assert_eq!(7373, c.high);
        assert_eq!(8192, c.max);

        assert!(c.should_verify(0u64));
        assert!(c.should_verify(256u64));
        assert!(!c.should_verify(4096u64));
        assert!(c.should_verify(7900u64));
        assert!(c.should_verify(8191u64));
        assert!(!c.should_verify(8192u64));
        assert!(!c.should_verify(8193u64));
    }

    #[test]
    fn test_coverage_distribution_from_0() {
        let c = CoverageFilter::from(8192, &Percent::new(0f32).unwrap());
        assert_eq!(1, c.low);
        assert_eq!(8191, c.high);
        assert_eq!(8192, c.max);

        assert!(c.should_verify(0u64));
        assert!(!c.should_verify(1u64));
        assert!(!c.should_verify(8190u64));
        assert!(c.should_verify(8191u64));
        assert!(!c.should_verify(8192u64));
        assert!(!c.should_verify(8193u64));
    }
    #[test]
    fn test_coverage_distribution_from_empty() {
        let c = CoverageFilter::empty();
        assert!(!c.should_verify(0u64));
        assert!(!c.should_verify(1u64));
        assert!(!c.should_verify(1024u64));
    }
}
