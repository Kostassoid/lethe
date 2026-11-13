use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone)]
pub enum Stage {
    Fill {
        value: u8,
    },
    Random,
    /// Fills each `step`-byte block with its own sequential number.
    #[allow(dead_code)] // part of the stage vocabulary, not used by built-in schemes yet
    Incremental {
        step: usize,
    },
}

impl Display for Stage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Stage::Fill { value } => write!(f, "fill with {value:#04X}"),
            Stage::Random => f.write_str("random fill"),
            Stage::Incremental { .. } => f.write_str("incremental fill"),
        }
    }
}

impl Stage {
    pub fn constant(value: u8) -> Stage {
        Stage::Fill { value }
    }

    pub fn zero() -> Stage {
        Self::constant(0)
    }

    pub fn one() -> Stage {
        Self::constant(0xff)
    }

    pub fn random() -> Stage {
        Stage::Random
    }

    #[allow(dead_code)] // part of the stage vocabulary, not used by built-in schemes yet
    pub fn incremental(block_size: usize) -> Stage {
        Stage::Incremental { step: block_size }
    }
}

#[derive(Debug, Clone)]
pub struct Scheme {
    pub description: String,
    pub stages: Vec<Stage>,
}

/// A catalogue of named sanitization schemes.
pub struct SchemeRepo {
    schemes: BTreeMap<&'static str, Scheme>,
}

impl SchemeRepo {
    pub fn all(&self) -> &BTreeMap<&'static str, Scheme> {
        &self.schemes
    }

    pub fn find(&self, name: &str) -> Option<&Scheme> {
        self.schemes.get(name)
    }
}

impl Default for SchemeRepo {
    fn default() -> SchemeRepo {
        let mut schemes = BTreeMap::new();

        schemes.insert(
            "none",
            Scheme {
                description: "No software passes (hardware erase only)".to_string(),
                stages: vec![],
            },
        );

        schemes.insert(
            "zero",
            Scheme {
                description: "Single zeroes fill".to_string(),
                stages: vec![Stage::zero()],
            },
        );

        schemes.insert(
            "random",
            Scheme {
                description: "Single random fill".to_string(),
                stages: vec![Stage::random()],
            },
        );

        schemes.insert(
            "random2x",
            Scheme {
                description: "Double random fill".to_string(),
                stages: vec![Stage::random(), Stage::random()],
            },
        );

        schemes.insert(
            "badblocks",
            Scheme {
                description: "Inspired by a badblocks tool -w action.".to_string(),
                stages: vec![
                    Stage::constant(0xaa),
                    Stage::constant(0x55),
                    Stage::constant(0xff),
                    Stage::constant(0x00),
                ],
            },
        );

        schemes.insert(
            "gost",
            Scheme {
                description: "GOST R 50739-95 (fake)".to_string(),
                stages: vec![Stage::zero(), Stage::random()],
            },
        );

        schemes.insert(
            "dod",
            Scheme {
                description: "DoD 5220.22-M / CSEC ITSG-06 / NAVSO P-5239-26".to_string(),
                stages: vec![Stage::zero(), Stage::one(), Stage::random()],
            },
        );

        schemes.insert(
            "vsitr",
            Scheme {
                description: "VSITR / RCMP TSSIT OPS-II".to_string(),
                stages: vec![
                    Stage::zero(),
                    Stage::one(),
                    Stage::zero(),
                    Stage::one(),
                    Stage::zero(),
                    Stage::one(),
                    Stage::random(),
                ],
            },
        );

        SchemeRepo { schemes }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_scheme_find() {
        let repo = SchemeRepo::default();

        assert!(repo.find("missing").is_none());

        let scheme = repo.find("random");
        assert!(scheme.is_some());

        let none = repo.find("none").unwrap();
        assert!(none.stages.is_empty());
    }
}
