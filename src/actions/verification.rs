use anyhow::Result;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Verification {
    No,
    Last(f32),
    All(f32),
}

impl Display for Verification {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Verification::No => f.write_str("No"),
            Verification::Last(c) => f.write_str(&format!("Last stage only ({}% coverage)", c)),
            Verification::All(c) => f.write_str(&format!("After each stage ({}% coverage)", c)),
        }
    }
}

impl Verification {
    fn should_verify(&self, current: u64, total: u64) -> bool {
        true
    }

    fn next_verifiable(&self, current: u64, total: u64) -> Result<u64> {
        Ok(current)
    }
}
