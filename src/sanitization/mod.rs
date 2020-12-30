pub mod stage;
pub use stage::*;

pub mod mem;

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Scheme {
    pub description: String,
    pub stages: Vec<Stage>,
}

pub struct SchemeRepo {
    schemes: BTreeMap<&'static str, Scheme>,
}

impl SchemeRepo {
    pub fn new(schemes: BTreeMap<&'static str, Scheme>) -> SchemeRepo {
        SchemeRepo { schemes }
    }

    pub fn default() -> SchemeRepo {
        let mut schemes = BTreeMap::new();

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

        Self::new(schemes)
    }

    pub fn all(&self) -> &BTreeMap<&'static str, Scheme> {
        &self.schemes
    }

    pub fn find(&self, name: &str) -> Option<&Scheme> {
        self.schemes.get(name)
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
    }
}
