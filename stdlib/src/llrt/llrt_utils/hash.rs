// Redistributed from LLRT; module paths and backend cfgs adapted by scripts/import-stdlib.py.
use std::hash::{DefaultHasher, Hash, Hasher};

#[inline]
pub fn default_hash<T: Hash + ?Sized>(v: &T) -> usize {
    let mut state = DefaultHasher::default();
    v.hash(&mut state);
    state.finish() as usize
}
