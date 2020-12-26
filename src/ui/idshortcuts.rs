use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub struct Tree {
    prefix: String,
    children: Vec<Tree>,
    value: Option<String>,
}

impl Tree {
    fn node(s: String) -> Tree {
        Tree {
            prefix: s,
            children: vec![],
            value: None,
        }
    }

    fn leaf(s: String) -> Tree {
        Tree {
            prefix: "".to_owned(),
            children: vec![],
            value: Some(s),
        }
    }

    fn add_child(&mut self, child: Tree) {
        self.children.push(child);
    }

    fn collect(&self, prefix: &str, r: &mut HashMap<String, String>) {
        if let Some(v) = &self.value {
            r.insert(prefix.to_owned(), v.to_owned());
        }

        let skip = self.children.len() < 2;

        for x in &self.children {
            let mut next_prefix = prefix;

            let p = format!("{}{}", prefix, x.prefix);
            if !skip {
                next_prefix = &p;
            }

            x.collect(next_prefix, r);
        }
    }
}

pub struct IdShortcuts {
    inner: HashMap<String, String>,
}

impl IdShortcuts {
    pub fn from(ids: HashSet<&str>) -> IdShortcuts {
        IdShortcuts {
            inner: Self::build_map_from(ids.into_iter().collect()),
        }
    }

    #[allow(dead_code)]
    pub fn keys(&self) -> Vec<&str> {
        self.inner.keys().map(|s| s.as_ref()).collect()
    }

    pub fn get_short(&self, key: &str) -> Option<&String> {
        self.inner.iter().find(|kv| kv.1 == key).map(|kv| kv.0)
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.inner
            .iter()
            .find(|kv| kv.0 == key || kv.1 == key)
            .map(|kv| kv.1)
    }

    fn build_map_from(ids: Vec<&str>) -> HashMap<String, String> {
        let mut root = Tree::node("".to_owned());
        for x in ids {
            Self::build_prefix_tree(&mut root, x, 1);
        }

        let mut m = HashMap::new();
        root.collect("", &mut m);
        m
    }

    fn build_prefix_tree(node: &mut Tree, id: &str, depth: usize) {
        if depth > id.len() {
            node.add_child(Tree::leaf(id.to_owned()));
        } else {
            let next_prefix = &id[depth - 1..depth];

            for n in &mut node.children {
                if n.prefix == next_prefix {
                    Self::build_prefix_tree(n, id, depth + 1);
                    return;
                }
            }

            let mut next = Tree::node(next_prefix.to_owned());
            Self::build_prefix_tree(&mut next, id, depth + 1);
            node.add_child(next);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn test_no_intersection() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec!["abc", "def", "ghi"].iter().cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(vec!("a", "d", "g"), sorted);
    }

    #[test]
    fn test_last_part() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec!["abc1", "abc2", "abc123"].iter().cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(vec!("1", "12", "2"), sorted);
    }

    #[test]
    fn test_normal() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec!["abc", "acd", "abd", "bac", "bad"].iter().cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(vec!("abc", "abd", "ac", "bc", "bd"), sorted);
    }

    #[test]
    fn test_sub_prefixes() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec!["abc", "abc1", "abc2", "abc23", "abc123"]
                .iter()
                .cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(vec!("", "1", "12", "2", "23"), sorted); //todo: this
    }

    #[test]
    fn test_real_windows() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec![
                "\\Device\\Harddisk0\\Partition1",
                "\\Device\\Harddisk0\\Partition2",
                "\\Device\\Harddisk0\\Partition3",
                "\\Device\\Harddisk0\\Partition4",
                "\\Device\\Harddisk1\\Partition1",
                "\\Device\\Harddisk2\\Partition1",
                "\\Device\\Harddisk2\\Partition2",
                "\\Device\\Harddisk4\\Partition1",
                "\\Device\\Harddisk4\\Partition2",
                "\\\\.\\PhysicalDrive0",
                "\\\\.\\PhysicalDrive1",
                "\\\\.\\PhysicalDrive2",
                "\\\\.\\PhysicalDrive3",
                "\\\\.\\PhysicalDrive4",
            ]
            .iter()
            .cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(
            vec!(
                "D01", "D02", "D03", "D04", "D1", "D21", "D22", "D41", "D42", "\\0", "\\1", "\\2",
                "\\3", "\\4"
            ),
            sorted
        );
        assert_eq!("\\\\.\\PhysicalDrive2", ids.get("\\2").unwrap());
        assert_eq!("\\Device\\Harddisk2\\Partition2", ids.get("D22").unwrap());
    }

    #[test]
    fn test_real_nix() {
        let ids = IdShortcuts::from(HashSet::from_iter(
            vec![
                "/dev/rdisk0s1",
                "/dev/rdisk0s3",
                "/dev/rdisk0s4",
                "/dev/rdisk2",
                "/dev/rdisk2s1",
            ]
            .iter()
            .cloned(),
        ));

        let mut sorted = ids.keys();
        sorted.sort();
        assert_eq!(vec!("01", "03", "04", "2", "2s"), sorted);
        assert_eq!("/dev/rdisk2", ids.get("2").unwrap());
        assert_eq!("/dev/rdisk2", ids.get("/dev/rdisk2").unwrap());
        assert_eq!("/dev/rdisk0s3", ids.get("03").unwrap());
        assert_eq!("/dev/rdisk0s3", ids.get("/dev/rdisk0s3").unwrap());
    }
}
