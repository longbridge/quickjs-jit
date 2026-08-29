use std::collections::HashMap;

use super::FunctionKey;

pub(super) fn is_current_generation(generations: &HashMap<u64, u64>, key: FunctionKey) -> bool {
    generations.get(&key.id) == Some(&key.generation)
}
