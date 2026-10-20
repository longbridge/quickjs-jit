// Vendored from LLRT d9fd873e793c (libs/llrt_utils/src/hash.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
use std::hash::{DefaultHasher, Hash, Hasher};

#[inline]
pub fn default_hash<T: Hash + ?Sized>(v: &T) -> usize {
    let mut state = DefaultHasher::default();
    v.hash(&mut state);
    state.finish() as usize
}
