use crate::storage::StorageRef;
use crate::ui::idshortcuts::IdShortcuts;
use std::collections::HashSet;

pub struct StorageRepo {
    raw: Vec<StorageRef>,
    refs: IdShortcuts,
}

impl StorageRepo {
    pub fn from(source: Vec<StorageRef>) -> Self {
        let flat_id_set: HashSet<String> = source
            .iter()
            .map(|r| {
                let mut rv = vec![r.id.clone()];
                rv.append(&mut r.children.iter().map(|c| c.id.clone()).collect());
                rv
            })
            .flatten()
            .collect::<HashSet<String>>()
            .to_owned();

        //todo: simplify!
        Self {
            raw: source,
            refs: IdShortcuts::from(flat_id_set.to_owned().iter().map(|x| x.as_str()).collect()),
        }
    }

    pub fn devices(&self) -> &[StorageRef] {
        return self.raw.as_slice();
    }

    pub fn get_short_id(&self, id: &str) -> Option<&String> {
        return self.refs.get_short(id);
    }

    pub fn find_by_id(&self, id: &str) -> Option<&StorageRef> {
        let canonical_id = self.refs.get(id).map(|s| s.as_str()).unwrap_or(id);
        self.raw.iter().find_map(|r| {
            if r.id == canonical_id {
                Some(r)
            } else {
                r.children.iter().find(|c| c.id == canonical_id)
            }
        })
    }
}
