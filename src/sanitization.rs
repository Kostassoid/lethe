use std::collections::HashMap;
use rand::prelude::*;
use rand::SeedableRng;
use streaming_iterator::StreamingIterator;

#[derive(Debug)]
pub enum SantitizationStage {
    Fill { value: u8 },
    Random { seed: u64 }
}

#[derive(Debug)]
pub struct StreamState {
    total_size: u64,
    block_size: usize,
    position: u64,
    buf: Vec<u8>,
    current_block_size: usize,
    eof: bool
}

#[derive(Debug)]
pub enum StreamKind {
    Fill,
    Random { gen: StdRng }
}

#[derive(Debug)]
pub struct SanitizationStream {
    kind: StreamKind,
    state: StreamState
}

impl SantitizationStage {
    pub fn zero() -> SantitizationStage { 
        SantitizationStage::Fill { value: 0x00 } 
    }

    pub fn one() -> SantitizationStage { 
        SantitizationStage::Fill { value: 0xff } 
    }

    pub fn random(seed: u64) -> SantitizationStage { 
        SantitizationStage::Random { seed }
    }
}

impl SanitizationStream {
    pub fn new(stage: &SantitizationStage, total_size: u64, block_size: usize) -> SanitizationStream {
        let (kind, buf) = match stage {
            SantitizationStage::Fill { value } => {
                let buf = vec![*value; block_size];
                (StreamKind::Fill, buf)
            },
            SantitizationStage::Random { seed } => {
                let buf = vec![0; block_size];
                let gen = SeedableRng::seed_from_u64(*seed); 
                (StreamKind::Random { gen }, buf)
            }
        };

        let state = StreamState {
            total_size, 
            block_size, 
            position: 0, 
            buf,
            eof: false,
            current_block_size: 0
        };
        SanitizationStream { kind, state }
    }
}

impl StreamingIterator for SanitizationStream {
    type Item = [u8];

    fn advance(&mut self) {
        if !self.state.eof && self.state.position < self.state.total_size {
            let chunk_size = std::cmp::min(
                self.state.block_size as u64, 
                self.state.total_size - self.state.position) as usize;

            println!("!!! stream out chunk_size: {}", chunk_size);

            match &mut self.kind {
                StreamKind::Fill => (),
                StreamKind::Random { gen } =>
                    gen.fill_bytes(&mut self.state.buf)
            };

            self.state.current_block_size = chunk_size;
            self.state.position += chunk_size as u64;
        } else {
            self.state.eof = true;
        }
    }

    fn get(&self) -> Option<&Self::Item> {
        if !self.state.eof {
            Some(&self.state.buf[..self.state.current_block_size as usize])
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct Scheme {
    stages: Vec<SantitizationStage>
}

impl Scheme {
    pub fn build_stages(&self) -> &Vec<SantitizationStage> {
        &self.stages //TODO: randomize PRNG seed values
    }
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

    const TEST_SIZE: u64 = 10245;
    const TEST_BLOCK: usize = 256;

    #[test]
    fn test_stage_fill_behaves() {
        let mut data1 = create_test_vec();
        let mut stage = SantitizationStage::Fill { value: 0x33 };

        fill(&mut data1, &mut stage);
        assert!(data1.iter().find(|x| **x != 0x33).is_none());

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
            .filter(|t| t.0 == t.1).count() as u64;

        assert!(unchanged < TEST_SIZE / 100); // allows for some edge cases

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
        let mut stream = SanitizationStream::new(stage, TEST_SIZE, TEST_BLOCK);

        let mut position = 0;
        while let Some(chunk) = stream.next() {
            let chunk_size = chunk.len();
            v[position..position + chunk_size].clone_from_slice(chunk);
            position += chunk_size;
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
