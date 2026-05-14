use rand::RngCore;
use rand::SeedableRng;
pub use streaming_iterator::StreamingIterator;

use crate::sanitization::mem::*;
use crate::sanitization::scheme::Stage;

const RANDOM_SEED_SIZE: usize = 32;
type RandomGenerator = rand_chacha::ChaCha8Rng;

#[derive(Debug)]
enum StreamGenerator {
    Fill,
    Random { gen: RandomGenerator },
    Incremental { step: usize, position: u64 },
}

pub struct Stream {
    generator: StreamGenerator,
    total_size: u64,
    pub position: u64,
    last_position: u64,
    buf: AlignedBuffer,
}

impl Stream {
    pub fn from(stage: &Stage, total_size: u64, block_size: usize) -> Self {
        let mut buf = AlignedBuffer::new(block_size, block_size);

        let generator = match stage {
            Stage::Fill { value } => {
                buf.fill(*value);
                StreamGenerator::Fill
            }
            Stage::Random => {
                let mut seed: [u8; RANDOM_SEED_SIZE] = [0; RANDOM_SEED_SIZE];
                rand::thread_rng().fill_bytes(&mut seed[..]);
                let mut gen = RandomGenerator::from_seed(seed);
                gen.set_word_pos(0);
                StreamGenerator::Random { gen }
            }
            Stage::Incremental { step } => StreamGenerator::Incremental {
                step: *step,
                position: 0,
            },
        };

        Stream {
            generator,
            total_size,
            position: 0,
            last_position: 0,
            buf,
        }
    }

    pub fn eof(&self) -> bool {
        self.position >= self.total_size && self.position == self.last_position
    }

    pub fn seek(&mut self, position: u64) {
        self.position = position;
        if self.position > self.total_size {
            self.position = self.total_size;
        }

        self.last_position = self.position;

        match &mut self.generator {
            StreamGenerator::Fill => (),
            StreamGenerator::Random { gen } => gen.set_word_pos((position >> 2) as u128),
            StreamGenerator::Incremental {
                step: _step,
                position: gen_position,
            } => *gen_position = position,
        };
    }
}

impl StreamingIterator for Stream {
    type Item = [u8];

    fn advance(&mut self) {
        if self.eof() {
            return;
        }

        match &mut self.generator {
            StreamGenerator::Fill => (),
            StreamGenerator::Random { gen } => gen.fill_bytes(self.buf.as_mut_slice()),
            StreamGenerator::Incremental { step, position } => {
                self.buf.fill(((*position / *step as u64) % 256) as u8);
                *position = *position + *step as u64;
            }
        };

        self.last_position = self.position;

        self.position += self.buf.len() as u64;
        if self.position > self.total_size {
            self.position = self.total_size;
        }
    }

    fn get(&self) -> Option<&Self::Item> {
        if self.eof() {
            None
        } else {
            let chunk_size = self.position - self.last_position;
            Some(&self.buf.as_mut_slice()[..chunk_size as usize])
        }
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
        let mut stream = Stream::from(&Stage::constant(0x33), TEST_SIZE, TEST_BLOCK);

        fill(&mut data1, &mut stream);
        
        assert!(data1.iter().find(|x| **x != 0x33).is_none());

        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stream);

        assert_eq!(data1, data2);
    }

    #[test]
    fn test_stage_random_behaves() {
        let mut data1 = create_test_vec();
        let mut stream =  Stream::from(&Stage::random(), TEST_SIZE, TEST_BLOCK);

        fill(&mut data1, &mut stream);

        assert_ne!(data1, create_test_vec());

        let unchanged = data1
            .iter()
            .zip(create_test_vec().iter())
            .filter(|t| t.0 == t.1)
            .count() as u64;

        assert!(unchanged < TEST_SIZE / 100); // allows for some edge cases

        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stream);

        assert_eq!(data1, data2);

        let mut stream3 = Stream::from(&Stage::random(), TEST_SIZE, TEST_BLOCK);
        let mut data3 = create_test_vec();
        fill(&mut data3, &mut stream3);

        assert_ne!(data3, data2);
    }

    #[test]
    fn test_stage_random_entropy() {
        let mut data = create_test_vec();
        let mut stage = Stream::from(&Stage::random(), TEST_SIZE, TEST_BLOCK);
        fill(&mut data, &mut stage);

        let source_entropy = calculate_entropy(create_test_vec().as_ref());
        let stage_entropy = calculate_entropy(data.as_ref());

        assert!(stage_entropy > source_entropy);
        assert!(stage_entropy > 0.9);
    }

    #[test]
    fn test_stage_incremental_behaves() {
        let mut data1 = create_test_vec();
        let mut stream = Stream::from(&Stage::incremental(TEST_BLOCK), TEST_SIZE, TEST_BLOCK);

        fill(&mut data1, &mut stream);

        for block in 0..TEST_SIZE / TEST_BLOCK as u64 {
            assert!(data1
                .iter()
                .skip(block as usize * TEST_BLOCK)
                .take(TEST_BLOCK)
                .find(|x| **x != (block % 256) as u8)
                .is_none());
        }

        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stream);

        assert_eq!(data1, data2);
    }

    fn create_test_vec() -> Vec<u8> {
        (0..TEST_SIZE).map(|x| (x % 256) as u8).collect()
    }

    fn fill(v: &mut Vec<u8>, stream: &mut Stream) -> () {
        let mut position = 0;
        stream.seek(0);

        while let Some(chunk) = stream.next() {
            let chunk_size = chunk.len();
            v[position..position + chunk_size].clone_from_slice(chunk);
            position += chunk_size;
        }
    }

    fn calculate_entropy(v: &[u8]) -> f64 {
        use flate2::{write::ZlibEncoder, Compression};
        use std::io::Write;

        let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
        e.write_all(v).unwrap();
        let compressed_bytes = e.finish();
        compressed_bytes.unwrap().len() as f64 / v.len() as f64
    }
}
