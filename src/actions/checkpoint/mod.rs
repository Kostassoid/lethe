use crate::sanitization::Scheme;
use crate::actions::{WipeTask, WipeState};
use blake2::{Blake2b, Digest};
use uuid::Uuid;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::io::{BufReader, BufWriter};
use std::fs::File;
use serde::{Serialize, Deserialize};
use chrono::{Utc, DateTime};

type Fingerprint = [u8; 32];
type IoResult<A> = std::io::Result<A>;

fn resolve_data_path() -> String {
    let root = std::env::var("XDG_DATA_HOME")
        .unwrap_or("~/.local/share".to_owned());

    format!("{}/lethe", &root)
}

fn calculate_fingerprint(sample: &[u8]) -> Fingerprint {
    let mut fingerprint: Fingerprint = Default::default();
    let hash = Blake2b::digest(sample);
    fingerprint.copy_from_slice(&hash[..32]);
    fingerprint
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Checkpoint {
    id: Uuid,
    //#[serde(with = "serde_instant")]
    timestamp: DateTime<Utc>,
    total_size: u64,
    block_size: usize,
    scheme: Scheme,
    stage: usize,
    at_verification: bool,
    position: u64,
    fingerprint: Fingerprint
}

impl Checkpoint {
    pub fn new(task: &WipeTask, state: &WipeState, sample: &[u8]) -> Checkpoint {
        Checkpoint {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            total_size: task.total_size,
            block_size: task.block_size,
            scheme: task.scheme.clone(),
            stage: state.stage,
            at_verification: state.at_verification,
            position: state.position,
            fingerprint: calculate_fingerprint(sample)
        }
    }

    pub fn update(&mut self, state: &WipeState) -> () {
        self.timestamp = Utc::now();
        self.stage = state.stage;
        self.position = state.position;
        self.at_verification = state.at_verification;
    }
}

struct CheckpointStore {
    path: PathBuf,
    index: HashMap<Uuid, Checkpoint>
}

impl CheckpointStore {
    fn load_from<P: AsRef<Path>>(path: P) -> IoResult<Self> {
        let file = File::open(&path)?;
        let buffered_reader = BufReader::new(file);
        //let serialized:Vec<u8> = Vec::new();
        //buffered_reader.read_to_end(&serialized)?;

        let list: Vec<Checkpoint> = serde_json::from_reader(buffered_reader).unwrap();

        let mut index: HashMap<Uuid, Checkpoint> = HashMap::new();
        for c in list.iter() {
            index.insert(c.id, c.clone());
        }

        Ok(CheckpointStore { 
            path: path.as_ref().to_path_buf(),
            index
        })
    }

    fn default() -> IoResult<Self> {
        CheckpointStore::load_from(resolve_data_path())
    }

    fn find(self, total_size: u64, sample: &[u8]) -> Vec<Checkpoint> {
        let fingerprint = calculate_fingerprint(sample);
        self.index.values()
            .filter(|c| c.total_size == total_size && c.fingerprint == fingerprint)
            .cloned()
            .collect()
    }

    fn update(&mut self, checkpoint: Checkpoint) -> () {
        self.index.insert(checkpoint.id, checkpoint);
        ()
    }

    fn remove(&mut self, id: &Uuid) -> () {
        self.index.remove(id);
        ()
    }

    fn flush(self) -> IoResult<()> {
        let mut list: Vec<Checkpoint> = Vec::with_capacity(self.index.len());
        for (_, v) in self.index.iter() {
            list.push(v.clone());
        }

        let file = File::open(&self.path)?;
        let buffered_writer = BufWriter::new(file);

        //let serialized = serde_json::to_string(&list).unwrap();

        serde_json::to_writer(buffered_writer, &list).unwrap();
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_resolve_data_path() {
        assert_eq!(resolve_data_path(), "~/.local/share/lethe");
    }

    #[test]
    fn test_fingerprint_calculation() {

        let sample1 = [0u8; 128];
        let sample2 = [0xffu8; 128];

        assert_eq!(calculate_fingerprint(&sample1), calculate_fingerprint(&sample1));
        assert_eq!(calculate_fingerprint(&sample2), calculate_fingerprint(&sample2));
        assert_ne!(calculate_fingerprint(&sample1), calculate_fingerprint(&sample2));
    }
}