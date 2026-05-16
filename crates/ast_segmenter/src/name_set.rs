use ahashmap::{AHashMap, AHashSet};

pub struct NameSet<K, V> {
    names: AHashMap<K, AHashSet<V>>,
}

impl<K, V> NameSet<K, V> {
    pub fn new() -> Self {
        Self {
            names: Default::default(),
        }
    }

    pub fn names(self) -> AHashMap<K, AHashSet<V>> {
        self.names
    }
}
impl<K: Eq + std::hash::Hash, V: Eq + std::hash::Hash> NameSet<K, V> {
    pub fn insert(&mut self, key: K, value: V) {
        self.names.entry(key).or_default().insert(value);
    }

    pub fn insert_all(&mut self, key: K, values: impl IntoIterator<Item = V>) {
        let entry = self.names.entry(key).or_default();
        for value in values {
            entry.insert(value);
        }
    }

    pub fn entry(&mut self, key: K) -> ahashmap::hash_map::Entry<'_, K, AHashSet<V>> {
        self.names.entry(key)
    }
}
impl<K: Default, V> Default for NameSet<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
