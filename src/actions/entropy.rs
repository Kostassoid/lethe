use crate::sanitization::mem::AlignedBuffer;
use crate::storage::StorageAccess;
use anyhow::{Context, Result};
use std::ops::Range;

pub const MID_ENTROPY_MIN: f64 = 1.0;
pub const MID_ENTROPY_MAX: f64 = 7.0;
const GRAPH_WIDTH: usize = 80;
const MAX_DISPLAYED_REGIONS: usize = 20;

#[derive(Debug)]
pub struct EntropyScan {
    pub offset: u64,
    pub block_size: usize,
    pub sample_size: usize,
    pub samples_per_block: usize,
    pub blocks_scanned: u64,
    pub candidate_blocks: u64,
    pub candidate_bytes: u64,
    pub region_count: u64,
    pub regions: Vec<Range<u64>>,
    pub selection: EntropySelection,
    pub graph: Vec<Option<f64>>,
}

#[derive(Debug)]
pub struct EntropySelection {
    offset: u64,
    block_size: usize,
    selected_blocks: Vec<bool>,
    candidate_bytes: u64,
}

impl EntropySelection {
    pub(crate) fn new(
        offset: u64,
        block_size: usize,
        selected_blocks: Vec<bool>,
        candidate_bytes: u64,
    ) -> Self {
        EntropySelection {
            offset,
            block_size,
            selected_blocks,
            candidate_bytes,
        }
    }

    pub(crate) fn is_selected(&self, position: u64) -> bool {
        if position < self.offset {
            return false;
        }
        self.selected_blocks
            .get(((position - self.offset) / self.block_size as u64) as usize)
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn selected_bytes(&self) -> u64 {
        self.candidate_bytes
    }
}

#[derive(Clone, Copy, Default)]
struct GraphBucket {
    entropy_sum: f64,
    samples: u64,
}

pub fn sample_entropy_with_progress(
    access: &mut dyn StorageAccess,
    total_size: u64,
    block_size: usize,
    offset: u64,
    io_alignment: usize,
    sample_size: usize,
    samples_per_block: usize,
    mut progress: impl FnMut(u64, u64),
) -> Result<EntropyScan> {
    if block_size == 0 || sample_size == 0 || samples_per_block == 0 {
        return Err(anyhow!(
            "Block size, sample size, and samples per block must be greater than zero."
        ));
    }
    if samples_per_block > 64 {
        return Err(anyhow!("Samples per block must be between 1 and 64."));
    }
    if offset >= total_size {
        return Err(anyhow!("Starting offset is greater than the storage size"));
    }
    if io_alignment == 0 || !io_alignment.is_power_of_two() {
        return Err(anyhow!("Invalid storage I/O alignment."));
    }
    if offset % io_alignment as u64 != 0 {
        return Err(anyhow!(
            "Starting offset must be aligned to the storage I/O alignment ({} bytes).",
            io_alignment
        ));
    }
    if block_size < io_alignment || block_size % io_alignment != 0 {
        return Err(anyhow!(
            "Sample block size must be a multiple of the storage I/O alignment ({} bytes).",
            io_alignment
        ));
    }

    let sample_size = sample_size.min(block_size);
    if sample_size < io_alignment || sample_size % io_alignment != 0 {
        return Err(anyhow!(
            "Sample size must be a multiple of the storage I/O alignment ({} bytes).",
            io_alignment
        ));
    }
    let available_positions = (block_size - sample_size) / io_alignment + 1;
    if samples_per_block > available_positions {
        return Err(anyhow!(
            "The requested number of samples exceeds the distinct aligned positions in an analysis block ({}).",
            available_positions
        ));
    }

    let remaining = total_size - offset;
    let blocks_scanned = remaining / block_size as u64
        + if remaining % block_size as u64 != 0 {
            1
        } else {
            0
        };
    let mut buckets = [GraphBucket::default(); GRAPH_WIDTH];
    let mut regions = Vec::new();
    let mut current_region_start = None;
    let mut candidate_blocks = 0;
    let mut candidate_bytes = 0;
    let mut region_count = 0;
    let mut selected_blocks = Vec::new();
    let mut blocks_processed = 0;
    let mut position = offset;
    let mut rng = rand::thread_rng();
    let mut counts = [0u64; 256];
    let buffer = AlignedBuffer::new(sample_size, io_alignment);
    let mut access_position = None;

    while position < total_size {
        let block_len = std::cmp::min(block_size as u64, total_size - position) as usize;
        let read_size = std::cmp::min(sample_size, block_len) / io_alignment * io_alignment;
        if read_size == 0 {
            return Err(anyhow!(
                "The final sample block is smaller than the storage I/O alignment."
            ));
        }

        let available_positions = (block_len - read_size) / io_alignment + 1;
        let sample_count = std::cmp::min(samples_per_block, available_positions);
        let mut sample_positions =
            rand::seq::index::sample(&mut rng, available_positions, sample_count)
                .into_iter()
                .map(|index| index * io_alignment)
                .collect::<Vec<_>>();
        sample_positions.sort_unstable();

        let mut sampled_bytes = 0u64;
        for sample_position in sample_positions {
            let sample_offset = position + sample_position as u64;
            if access_position != Some(sample_offset) {
                access
                    .seek(sample_offset)
                    .context("Unable to seek while sampling storage")?;
            }
            let bytes_read = access
                .read(&mut buffer.as_mut_slice()[..read_size])
                .context("Unable to read while sampling storage")?;
            if bytes_read != read_size {
                return Err(anyhow!(
                    "Short read while sampling at byte {}: expected {} bytes, got {}.",
                    position + sample_position as u64,
                    read_size,
                    bytes_read
                ));
            }
            access_position = Some(sample_offset + read_size as u64);

            for byte in &buffer.as_mut_slice()[..read_size] {
                counts[*byte as usize] += 1;
            }
            sampled_bytes += read_size as u64;
        }

        let entropy = shannon_entropy(&counts, sampled_bytes);
        counts.fill(0);

        let graph_index = (blocks_processed * GRAPH_WIDTH as u64 / blocks_scanned)
            .min((GRAPH_WIDTH - 1) as u64) as usize;
        buckets[graph_index].entropy_sum += entropy;
        buckets[graph_index].samples += 1;
        blocks_processed += 1;

        let block_end = position + block_len as u64;
        if (MID_ENTROPY_MIN..=MID_ENTROPY_MAX).contains(&entropy) {
            candidate_blocks += 1;
            candidate_bytes += block_len as u64;
            selected_blocks.push(true);
            if current_region_start.is_none() {
                current_region_start = Some(position);
            }
        } else if let Some(start) = current_region_start.take() {
            if regions.len() < MAX_DISPLAYED_REGIONS {
                regions.push(start..position);
            }
            region_count += 1;
            selected_blocks.push(false);
        } else {
            selected_blocks.push(false);
        }

        position = block_end;
        progress(position - offset, remaining);
    }

    if let Some(start) = current_region_start {
        if regions.len() < MAX_DISPLAYED_REGIONS {
            regions.push(start..total_size);
        }
        region_count += 1;
    }

    let graph = buckets
        .iter()
        .map(|bucket| {
            if bucket.samples == 0 {
                None
            } else {
                Some(bucket.entropy_sum / bucket.samples as f64)
            }
        })
        .collect();

    Ok(EntropyScan {
        offset,
        block_size,
        sample_size,
        samples_per_block,
        blocks_scanned,
        candidate_blocks,
        candidate_bytes,
        region_count,
        regions,
        selection: EntropySelection::new(offset, block_size, selected_blocks, candidate_bytes),
        graph,
    })
}

fn shannon_entropy(counts: &[u64; 256], total: u64) -> f64 {
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let probability = count as f64 / total as f64;
            -probability * probability.log2()
        })
        .sum()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::storage::StorageAccess;
    use anyhow::Result;
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};

    struct MemoryAccess {
        data: Cursor<Vec<u8>>,
    }

    impl StorageAccess for MemoryAccess {
        fn position(&mut self) -> Result<u64> {
            Ok(self.data.position())
        }

        fn seek(&mut self, position: u64) -> Result<u64> {
            Ok(self.data.seek(SeekFrom::Start(position))?)
        }

        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            Ok(self.data.read(buffer)?)
        }

        fn write(&mut self, data: &[u8]) -> Result<()> {
            self.data.write_all(data)?;
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn samples_blocks_and_selects_only_mid_entropy_regions() {
        let block_size = 8192;
        let mut data = vec![0u8; block_size];
        data.extend((0..block_size).map(|index| (index % 2) as u8));
        data.extend((0..block_size).map(|index| index as u8));
        let mut access = MemoryAccess {
            data: Cursor::new(data),
        };

        let scan = sample_entropy_with_progress(
            &mut access,
            (block_size * 3) as u64,
            block_size,
            0,
            512,
            4096,
            1,
            |_, _| {},
        )
        .unwrap();

        assert_eq!(scan.blocks_scanned, 3);
        assert_eq!(scan.candidate_blocks, 1);
        assert_eq!(scan.regions, vec![8192..16384]);
        assert!(!scan.selection.is_selected(0));
        assert!(scan.selection.is_selected(8192));
        assert!(!scan.selection.is_selected(16384));
        assert_eq!(scan.graph.len(), GRAPH_WIDTH);
    }

    #[test]
    fn rejects_samples_that_do_not_match_device_alignment() {
        let mut access = MemoryAccess {
            data: Cursor::new(vec![0; 8192]),
        };

        assert!(
            sample_entropy_with_progress(&mut access, 8192, 4096, 0, 512, 768, 1, |_, _| {},)
                .is_err()
        );
    }
}
