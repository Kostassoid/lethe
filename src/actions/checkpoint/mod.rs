use crate::sanitization::Scheme;
use crate::actions::{WipeTask, WipeState};
use blake2::{Blake2b, Digest};
use uuid::Uuid;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::{read_dir, read_to_string, write, remove_file, create_dir_all};
use serde::{Serialize, Deserialize};
use chrono::{Utc, DateTime};

type Fingerprint = [u8; 32];
type IoResult<A> = std::io::Result<A>;

const CHECKPOINT_EXT: &str = ".checkpoint";

fn calculate_fingerprint(sample: &[u8]) -> Fingerprint {
    let mut fingerprint: Fingerprint = Default::default();
    let hash = Blake2b::digest(sample);
    fingerprint.copy_from_slice(&hash[..32]);
    fingerprint
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Checkpoint {
    id: Uuid,
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

#[derive(Debug, PartialEq, Eq)]
enum CheckpointOperation {
    Update(Checkpoint),
    Remove
}

#[derive(Debug, PartialEq, Eq)]
struct CheckpointStore {
    index: HashMap<Uuid, Checkpoint>,
    pending: HashMap<Uuid, CheckpointOperation>
}

impl CheckpointStore {
    fn new() -> Self {
        CheckpointStore { index: HashMap::new(), pending: HashMap::new() }
    }

    fn load_from<P: AsRef<Path>>(&mut self, path: P) -> IoResult<()> {
        create_dir_all(&path)?;

        let rd = read_dir(&path)?;
        let index = rd
            .filter_map(std::io::Result::ok)
            .map(|de| de.path())
            .filter(|path| path.to_str().unwrap().ends_with(CHECKPOINT_EXT))
            .flat_map(read_to_string)
            .flat_map(|json| serde_json::from_str::<Checkpoint>(&json))
            .map(|cp| (cp.id, cp))
            .collect::<HashMap<_, _>>();

        self.index = index;
        Ok(())
    }

    fn find(self, total_size: u64, sample: &[u8]) -> Vec<Checkpoint> {
        let fingerprint = calculate_fingerprint(sample);
        self.index.values()
            .filter(|c| c.total_size == total_size && c.fingerprint == fingerprint)
            .cloned()
            .collect()
    }

    fn update(&mut self, checkpoint: Checkpoint) -> () {
        self.pending.insert(checkpoint.id.clone(), CheckpointOperation::Update(checkpoint.clone()));
        self.index.insert(checkpoint.id, checkpoint);
        ()
    }

    fn remove(&mut self, id: &Uuid) -> () {
        self.pending.insert(id.clone(), CheckpointOperation::Remove);
        self.index.remove(id);
        ()
    }

    fn flush<P: AsRef<Path>>(&mut self, path: P) -> IoResult<()> {
        std::fs::create_dir_all(&path)?;
        
        for (id, op) in self.pending.iter() {
            let file_path = path.as_ref().join(format!("{}{}", id, CHECKPOINT_EXT));

            match op {
                CheckpointOperation::Update(cp) => write(file_path, serde_json::to_string(cp).unwrap())?,
                CheckpointOperation::Remove => remove_file(file_path)?
            };
        }

        self.pending.clear();

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::sanitization::SchemeRepo;
    use crate::actions::{WipeTask, WipeState, Verify};
    use assert_matches::assert_matches;

    // #[test]
    // fn test_resolve_data_path() {
    //     assert_eq!(resolve_data_path(), "~/.local/share/lethe");
    // }

    #[test]
    fn test_fingerprint_calculation() {

        let sample1 = [0u8; 128];
        let sample2 = [0xffu8; 128];

        assert_eq!(calculate_fingerprint(&sample1), calculate_fingerprint(&sample1));
        assert_eq!(calculate_fingerprint(&sample2), calculate_fingerprint(&sample2));
        assert_ne!(calculate_fingerprint(&sample1), calculate_fingerprint(&sample2));
    }

    #[test]
    fn test_checkpoint_store_save_load() {

        let dir = tempfile::tempdir().unwrap();
        //let dir = "/Users/kostassoid/proj/tmp/lethe";

        let mut new_store = CheckpointStore::new();

        let cp1 = create_checkpoint(&[0x11u8; 128]);
        new_store.update(cp1);

        let cp2 = create_checkpoint(&[0x22u8; 128]);
        new_store.update(cp2);

        let cp3 = create_checkpoint(&[0x33u8; 128]);
        new_store.update(cp3);

        new_store.flush(&dir).unwrap();

        let mut loaded_store = CheckpointStore::new();
        loaded_store.load_from(&dir).unwrap();

        assert_eq!(&new_store, &loaded_store);
    }

    #[test]
    fn test_checkpoint_store_basic_operations() {
        let mut store = CheckpointStore::new();

        let sample = [0x67u8; 128];

        let mut cp1 = create_checkpoint(&sample);
        let cp1id = cp1.id.clone();

        let cp2 = create_checkpoint(&sample);
        let cp2id = cp2.id.clone();

        store.update(cp1.clone());

        assert_eq!(store.index.len(), 1);
        assert_eq!(store.pending.len(), 1);

        cp1.position = 1000;
        store.update(cp1.clone());

        assert_eq!(store.index.len(), 1);
        assert_eq!(store.index.get(&cp1id), Some(&cp1));

        assert_eq!(store.pending.len(), 1);
        assert_eq!(store.pending.get(&cp1id), Some(&CheckpointOperation::Update(cp1.clone())));

        store.update(cp2.clone());

        assert_eq!(store.index.len(), 2);
        assert_eq!(store.pending.len(), 2);

        store.remove(&cp1id);        

        assert_eq!(store.index.len(), 1);
        assert_eq!(store.index.get(&cp1id), None);
        assert_eq!(store.index.get(&cp2id), Some(&cp2));

        assert_eq!(store.pending.len(), 2);
        assert_eq!(store.pending.get(&cp1id), Some(&CheckpointOperation::Remove));
        assert_eq!(store.pending.get(&cp2id), Some(&CheckpointOperation::Update(cp2.clone())));
    }

    fn create_checkpoint(sample: &[u8]) -> Checkpoint {
        let repo = SchemeRepo::default();
        let scheme = repo.find("random2").unwrap();
        let task = WipeTask::new(scheme.clone(), Verify::All, 12345000, 4096);
        let state = WipeState { stage: 1, at_verification: true, position: 0 };
        Checkpoint::new(&task, &state, &sample)
    }
}