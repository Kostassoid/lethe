use anyhow::Result;
use std::fmt::{Display, Formatter};
use std::ops::Range;

#[derive(Debug)]
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
}

#[derive(Debug)]
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

impl Verification {}

struct VerificationMap {
    range: Vec<Range<u64>>,
}

impl VerificationMap {
    fn from(total: u64, coverage: Percent) -> VerificationMap {
        let mut r = Vec::<Range<u64>>::new();

        r.push(0..total);

        VerificationMap { range: r }
    }

    fn should_verify(&self, current: u64) -> bool {
        self.range.iter().any(|r| r.contains(&current))
    }

    fn next_verifiable(&self, current: u64) -> Option<u64> {
        self.range
            .iter()
            .find(|r| r.start >= current && r.end < current)
            .map(|r| r.start)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_matches::*;

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
}
