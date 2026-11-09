use std::ops::Range;

use rand::{Rng, SeedableRng};
use streaming_iterator::StreamingIterator;

use crate::sanitization::mem::AlignedBuffer;
use crate::sanitization::scheme::Stage;

pub(crate) const RANDOM_SEED_SIZE: usize = 32;
/// Extra bytes generated per chunk so that random data can be reproduced starting at any
/// byte position, even a non-word-aligned one.
const RANDOM_ALIGN_SLACK: usize = 3;
type RandomGenerator = rand_chacha::ChaCha8Rng;

/// Produces the sanitization pattern of a single [`Stage`], chunk by chunk, over a fixed
/// area of the storage.
///
/// The stream is deterministic: for any given seed, the bytes generated for a device
/// position never change, regardless of where iteration started. This is what makes
/// verification of random passes possible.
///
/// Chunks are aligned to the absolute block grid of the device; the final chunk of the
/// area may be shorter than the block size.
pub struct Stream {
    generator: StreamGenerator,
    buf: AlignedBuffer,
    /// Absolute device position where the stream stops (exclusive).
    end: u64,
    /// Absolute device position of the next chunk to generate.
    next: u64,
    /// Absolute device positions covered by the current chunk in `buf`.
    chunk: Range<u64>,
}

#[derive(Debug)]
#[expect(clippy::large_enum_variant)] // a single generator per stream; the RNG dominates
enum StreamGenerator {
    /// Buffer is filled once with a constant and never changes.
    Fill,
    Random {
        rng: RandomGenerator,
    },
    Incremental {
        step: usize,
    },
}

impl Stream {
    pub fn new(stage: &Stage, area: Range<u64>, block_size: usize) -> Self {
        let mut seed = [0u8; RANDOM_SEED_SIZE];
        rand::rng().fill_bytes(&mut seed);
        Self::with_seed(stage, area, block_size, seed)
    }

    /// Builds a stream with an explicit random seed.
    pub(crate) fn with_seed(
        stage: &Stage,
        area: Range<u64>,
        block_size: usize,
        seed: [u8; RANDOM_SEED_SIZE],
    ) -> Self {
        let mut buf = AlignedBuffer::new(block_size + RANDOM_ALIGN_SLACK, block_size);

        let generator = match stage {
            Stage::Fill { value } => {
                buf.fill(*value);
                StreamGenerator::Fill
            }
            Stage::Random => StreamGenerator::Random {
                rng: RandomGenerator::from_seed(seed),
            },
            Stage::Incremental { step } => StreamGenerator::Incremental { step: *step },
        };

        Stream {
            generator,
            buf,
            end: area.end,
            next: area.start,
            chunk: area.start..area.start,
        }
    }

    /// Repositions the stream so that the next chunk starts at `position`.
    pub fn seek(&mut self, position: u64) {
        let position = position.min(self.end);
        self.next = position;
        self.chunk = position..position;
    }

    fn generate(&mut self) {
        match &mut self.generator {
            StreamGenerator::Fill => {}
            StreamGenerator::Random { rng } => {
                // ChaCha word positions address 4-byte words; regenerate from the word
                // containing the chunk start, in case it is not word-aligned.
                rng.set_word_pos(u128::from(self.chunk.start / 4));
                rng.fill_bytes(self.buf.as_mut_slice());
            }
            StreamGenerator::Incremental { step } => {
                let block_number = self.chunk.start / *step as u64;
                let value = u8::try_from(block_number % 256).expect("value is modulo 256");
                self.buf.fill(value);
            }
        }
    }

    /// Offset of the chunk data within the buffer.
    fn chunk_offset(&self) -> usize {
        (self.chunk.start % 4) as usize
    }
}

impl StreamingIterator for Stream {
    type Item = [u8];

    fn advance(&mut self) {
        if self.next >= self.end {
            self.chunk = self.end..self.end;
            return;
        }

        let len = (self.end - self.next).min(self.buf.len() as u64 - RANDOM_ALIGN_SLACK as u64);
        self.chunk = self.next..self.next + len;
        self.next = self.chunk.end;

        self.generate();
    }

    fn get(&self) -> Option<&Self::Item> {
        if self.chunk.is_empty() {
            None
        } else {
            let offset = self.chunk_offset();
            let len = usize::try_from(self.chunk.end - self.chunk.start)
                .expect("chunk length exceeds addressable size");
            Some(&self.buf.as_slice()[offset..offset + len])
        }
    }
}

#[cfg(test)]
mod test {
    // Test sizes are tiny, so the narrowing casts below are always lossless.
    #![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

    use super::*;

    const TEST_SIZE: u64 = 10245;
    const TEST_BLOCK: usize = 256;

    #[test]
    fn test_stage_fill_behaves() {
        let mut data1 = create_test_vec();
        let mut stream = Stream::new(&Stage::constant(0x33), 0..TEST_SIZE, TEST_BLOCK);

        fill(&mut data1, &mut stream);

        assert!(data1.iter().find(|x| **x != 0x33).is_none());

        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stream);

        assert_eq!(data1, data2);
    }

    #[test]
    fn test_stage_random_behaves() {
        let mut data1 = create_test_vec();
        let mut stream = Stream::new(&Stage::random(), 0..TEST_SIZE, TEST_BLOCK);

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

        let mut stream3 = Stream::new(&Stage::random(), 0..TEST_SIZE, TEST_BLOCK);
        let mut data3 = create_test_vec();
        fill(&mut data3, &mut stream3);

        assert_ne!(data3, data2);
    }

    #[test]
    fn test_stage_random_is_seekable() {
        let mut stream = Stream::new(&Stage::random(), 0..TEST_SIZE, TEST_BLOCK);
        let mut from_start = create_test_vec();
        fill(&mut from_start, &mut stream);

        let mut from_middle = create_test_vec();
        stream.seek(TEST_SIZE / 2);
        let offset = TEST_SIZE as usize / 2;
        let mut position = offset;
        while let Some(chunk) = stream.next() {
            let chunk_size = chunk.len();
            from_middle[position..position + chunk_size].clone_from_slice(chunk);
            position += chunk_size;
        }

        assert_eq!(position, TEST_SIZE as usize);
        assert_eq!(&from_middle[offset..], &from_start[offset..]);
    }

    #[test]
    fn test_stage_random_matches_sequential_reference_from_any_offset() {
        let stage = Stage::random();
        let mut stream = Stream::new(&stage, 0..TEST_SIZE, TEST_BLOCK);

        let mut reference = create_test_vec();
        let mut position = 0;
        while let Some(chunk) = stream.next() {
            reference[position..position + chunk.len()].copy_from_slice(chunk);
            position += chunk.len();
        }
        assert_eq!(position, TEST_SIZE as usize);

        let offsets = [
            0,
            1,
            2,
            3,
            4,
            5,
            TEST_BLOCK as u64 - 1,
            TEST_BLOCK as u64,
            TEST_BLOCK as u64 + 1,
            2 * TEST_BLOCK as u64 + 3,
            TEST_SIZE / 2,
            TEST_SIZE - 3,
            TEST_SIZE - 2,
            TEST_SIZE - 1,
            TEST_SIZE,
        ];

        for offset in offsets {
            stream.seek(offset);

            let mut actual = vec![0u8; (TEST_SIZE - offset) as usize];
            let mut position = 0;
            while let Some(chunk) = stream.next() {
                actual[position..position + chunk.len()].copy_from_slice(chunk);
                position += chunk.len();
            }

            assert_eq!(position, actual.len());
            assert_eq!(
                actual,
                reference[offset as usize..],
                "stream mismatch after seeking to {offset}"
            );
        }
    }

    #[test]
    fn test_stage_random_single_chunk_matches_reference_at_arbitrary_offsets() {
        let stage = Stage::random();
        let mut stream = Stream::new(&stage, 0..TEST_SIZE, TEST_BLOCK);

        let mut reference = create_test_vec();
        let mut position = 0;
        while let Some(chunk) = stream.next() {
            reference[position..position + chunk.len()].copy_from_slice(chunk);
            position += chunk.len();
        }

        // Jump back and forth; each jump must regenerate exactly the bytes that
        // sequential generation produced at that absolute position.
        for offset in [
            TEST_SIZE / 3,
            7,
            TEST_SIZE - 5,
            TEST_SIZE / 2 + 1,
            3,
            TEST_BLOCK as u64 + 2,
            0,
            TEST_SIZE - 1,
        ] {
            stream.seek(offset);

            let Some(chunk) = stream.next() else {
                continue;
            };

            assert_eq!(
                chunk,
                &reference[offset as usize..offset as usize + chunk.len()],
                "chunk mismatch after seeking to {offset}"
            );
        }
    }

    #[test]
    fn test_stage_random_clamps_seek_past_the_end() {
        let mut stream = Stream::new(&Stage::random(), 100..TEST_SIZE, TEST_BLOCK);

        stream.seek(TEST_SIZE + 100);
        assert!(stream.next().is_none());

        stream.seek(TEST_SIZE);
        assert!(stream.next().is_none());
    }

    #[test]
    fn test_stage_random_entropy() {
        let mut data = create_test_vec();
        let mut stage = Stream::new(&Stage::random(), 0..TEST_SIZE, TEST_BLOCK);
        fill(&mut data, &mut stage);

        let source_entropy = calculate_entropy(create_test_vec().as_ref());
        let stage_entropy = calculate_entropy(data.as_ref());

        assert!(stage_entropy > source_entropy);
        assert!(stage_entropy > 0.9);
    }

    #[test]
    fn test_stage_incremental_behaves() {
        let mut data1 = create_test_vec();
        let mut stream = Stream::new(&Stage::incremental(TEST_BLOCK), 0..TEST_SIZE, TEST_BLOCK);

        fill(&mut data1, &mut stream);

        for block in 0..TEST_SIZE / TEST_BLOCK as u64 {
            assert!(
                data1
                    .iter()
                    .skip(block as usize * TEST_BLOCK)
                    .take(TEST_BLOCK)
                    .find(|x| **x != (block % 256) as u8)
                    .is_none()
            );
        }

        let mut data2 = create_test_vec();
        fill(&mut data2, &mut stream);

        assert_eq!(data1, data2);
    }

    fn create_test_vec() -> Vec<u8> {
        (0..TEST_SIZE).map(|x| (x % 256) as u8).collect()
    }

    fn fill(v: &mut [u8], stream: &mut Stream) {
        let mut position = 0;
        stream.seek(0);

        while let Some(chunk) = stream.next() {
            let chunk_size = chunk.len();
            v[position..position + chunk_size].clone_from_slice(chunk);
            position += chunk_size;
        }
    }

    fn calculate_entropy(v: &[u8]) -> f64 {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;

        let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
        e.write_all(v).unwrap();
        let compressed_bytes = e.finish();
        compressed_bytes.unwrap().len() as f64 / v.len() as f64
    }
}
