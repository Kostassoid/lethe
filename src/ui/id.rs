use std::collections::{HashMap, HashSet};

struct IdMapper {
    inner: HashMap<String, String>,
}

impl IdMapper {
    pub fn from(ids: HashSet<&str>) -> IdMapper {
        IdMapper {
            inner: Default::default(), // Self::build_map_from(ids.iter().collect()),
        }
    }

    pub fn all(&self) -> Vec<&str> {
        vec![""]
    }

    fn build_map_from(ids: Vec<&str>) -> HashMap<String, String> {
        panic!("todo")
    }

    fn build_from_prefix(ids: Vec<&str>, prefix: &str) -> HashMap<String, String> {
        panic!("todo")
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn test_no_intersection_means_noop() {
        let ids = IdMapper::from(HashSet::from_iter(
            vec!["abc", "def", "ghi"].iter().cloned(),
        ));

        assert_eq!(vec!("abc", "def", "ghi"), ids.all());
    }
}
