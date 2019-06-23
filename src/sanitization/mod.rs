#![cfg(feature = "std")]
extern crate rand;

use std::collections::HashMap;
use rand::prelude::*;
use rand::SeedableRng;

pub trait SanitizationStage {
    fn next(&mut self, size: u64, buffer: &mut [u8]) -> ();
    fn reset(&mut self) -> ();
}

#[derive(Debug, Clone)]
pub enum SchemeStage {
    Zero,
    One,
    Random { seed: u64, gen: StdRng }
}

impl SanitizationStage for SchemeStage {
    fn next(&mut self, size: u64, buffer: &mut [u8]) -> () {
        match &self {
            SchemeStage::Zero => (),
            SchemeStage::One => (),
            SchemeStage::Random { seed, gen } => { 
                let x: StdRng = SeedableRng::seed_from_u64(*seed);
                x.fill_bytes(buffer);
            }
        }
    }

    fn reset(&mut self) -> () {
        match &self {
            SchemeStage::Zero => (),
            SchemeStage::One => (),
            SchemeStage::Random { seed, gen } => gen.seed_from_u64(*seed)
        }
    }
}

#[derive(Debug, Clone)]
struct Scheme {
    stages: Vec<SchemeStage>
}

struct Schemes {
    schemes: HashMap<&'static str, Scheme>
}

impl Schemes {
    pub fn new(schemes: HashMap<&'static str, Scheme>) -> Schemes {
        Schemes { schemes }
    }

    pub fn default() -> Schemes {
        let mut schemes = HashMap::new();

        schemes.insert("zero", Scheme { stages: vec!(SchemeStage::Zero) });
        schemes.insert("one", Scheme { stages: vec!(SchemeStage::One) });
        schemes.insert("random", Scheme { stages: vec!(SchemeStage::Random { seed: 0 }) });
        
        Self::new(schemes)
    }

    pub fn all(&self) -> &HashMap<&'static str, Scheme> {
        &self.schemes
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_thread_rng() {
        assert_eq!(0, 0);
    }
}
