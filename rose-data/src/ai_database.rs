use rose_file_readers::AipFile;
use std::collections::HashMap;

/// Moves `map` into a `Vec` indexed by key, with `None` for absent keys, so lookups
/// done every tick are an index instead of a hash.
pub(crate) fn dense_from_map<T>(map: HashMap<u16, T>) -> Vec<Option<T>> {
    let len = map.keys().max().map_or(0, |&max| max as usize + 1);
    let mut dense = Vec::with_capacity(len);
    dense.resize_with(len, || None);
    for (key, value) in map {
        dense[key as usize] = Some(value);
    }
    dense
}

pub struct AiDatabase {
    strings: Vec<Option<String>>,
    aips: Vec<Option<AipFile>>,
}

impl AiDatabase {
    pub fn new(strings: HashMap<u16, String>, aips: HashMap<u16, AipFile>) -> Self {
        Self {
            strings: dense_from_map(strings),
            aips: dense_from_map(aips),
        }
    }

    pub fn get_ai(&self, index: usize) -> Option<&AipFile> {
        // The index is truncated to u16, as the former map key was.
        self.aips.get(index as u16 as usize)?.as_ref()
    }

    #[allow(dead_code)]
    pub fn get_ai_string(&self, index: usize) -> Option<&str> {
        self.strings.get(index as u16 as usize)?.as_deref()
    }
}
