use std::collections::HashMap;
use rand::prelude::*;
use rand::SeedableRng;
use nix::libc;
use std::mem;

#[derive(Debug, Clone)]
enum SantitizationStage {
    Fill { value: u8 },
    Random { seed: u64, gen: StdRng }
}

fn unsafe_fill(buffer: &mut [u8], value: u8) -> () {
    unsafe {
        libc::memset(
            buffer.as_mut_ptr() as _,
            value as i32,
            buffer.len() * mem::size_of::<u8>(),
        );
    }
}

impl SantitizationStage {
    pub fn zero() -> SantitizationStage { 
        SantitizationStage::Fill { value: 0x00 } 
    }

    pub fn one() -> SantitizationStage { 
        SantitizationStage::Fill { value: 0xff } 
    }

    pub fn random(seed: u64) -> SantitizationStage { 
        SantitizationStage::Random { seed, gen: SeedableRng::seed_from_u64(seed) }
    }

    fn next(&mut self, buffer: &mut [u8]) -> () {
        match self {
            SantitizationStage::Fill { ref value } => {
                unsafe_fill(buffer, *value);
            },
            SantitizationStage::Random { seed: _, ref mut gen } => { 
                &mut gen.fill_bytes(buffer);
            }
        }
    }

    fn reset(&mut self) -> () {
        match self {
            SantitizationStage::Fill { value: _ } => (),
            SantitizationStage::Random { seed, ref mut gen } => { 
                *gen = SeedableRng::seed_from_u64(*seed); 
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scheme {
    stages: Vec<SantitizationStage>
}

pub struct SchemeRepo {
    schemes: HashMap<&'static str, Scheme>
}

impl SchemeRepo {
    pub fn new(schemes: HashMap<&'static str, Scheme>) -> SchemeRepo {
        SchemeRepo { schemes }
    }

    pub fn default() -> SchemeRepo {
        let mut schemes = HashMap::new();

        schemes.insert("zero", Scheme { stages: vec!(
            SantitizationStage::zero()
        )});

        schemes.insert("one", Scheme { stages: vec!(
            SantitizationStage::one()
        )});

        schemes.insert("random", Scheme { stages: vec!(
            SantitizationStage::random(thread_rng().next_u64())
        )});
        
        Self::new(schemes)
    }

    pub fn all(&self) -> &HashMap<&'static str, Scheme> {
        &self.schemes
    }

    pub fn find(&self, name: &str) -> Option<&Scheme> {
        self.schemes.get(name)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const TEST_SIZE: usize = 10245;
    const TEST_BLOCK: usize = 256;

    #[test]
    fn test_stage_fill_behaves() {
        let mut data1 = create_test_vec();
        let mut stage = SantitizationStage::Fill { value: 0x33 };

        fill(&mut data1, &mut stage);
        assert!(data1.iter().find(|x| **x != 0x33).is_none());

        stage.reset();
        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stage);

        assert_eq!(data1, data2);
    }

    #[test]
    fn test_stage_random_behaves() {
        let mut data1 = create_test_vec();
        let mut stage = SantitizationStage::random(666);

        fill(&mut data1, &mut stage);

        assert_ne!(data1, create_test_vec());

        let unchanged = data1.iter().zip(create_test_vec().iter())
            .filter(|t| t.0 == t.1).count();

        assert!(unchanged < TEST_SIZE / 100); // allows for some edge cases

        stage.reset();
        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stage);
        
        assert_eq!(data1, data2);

        let mut stage3 = SantitizationStage::random(333);
        let mut data3 = create_test_vec();
        fill(&mut data3, &mut stage3);

        assert_ne!(data3, data2);
    }

    #[test]
    fn test_stage_random_entropy() {
        let mut data = create_test_vec();
        let mut stage = SantitizationStage::random(666);
        fill(&mut data, &mut stage);

        let source_entropy = calculate_entropy(create_test_vec().as_ref());
        let stage_entropy = calculate_entropy(data.as_ref());

        assert!(stage_entropy > source_entropy);
        assert!(stage_entropy > 0.9);
    }

    #[test]
    fn test_scheme_find() {
        let repo = SchemeRepo::default();

        assert!(repo.find("missing").is_none());

        let scheme = repo.find("one");
        assert!(scheme.is_some());
    }

    fn create_test_vec() -> Vec<u8> {
        (0..TEST_SIZE).map(|x| (x % 256) as u8).collect()
    }

    fn fill(v: &mut Vec<u8>, stage: &mut SantitizationStage) -> () {
        for ch in v.chunks_mut(TEST_BLOCK) {
            stage.next(ch)
        }
    }

    fn calculate_entropy(v: &[u8]) -> f64 {
        use std::io::Write;
        use flate2::{write::ZlibEncoder, Compression};

        let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
        e.write_all(v).unwrap();
        let compressed_bytes = e.finish();
        compressed_bytes.unwrap().len() as f64 / v.len() as f64
    }

}
