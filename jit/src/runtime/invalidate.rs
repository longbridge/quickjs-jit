use std::collections::HashMap;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::FunctionKey;

pub(super) fn is_current_generation(generations: &HashMap<u64, u64>, key: FunctionKey) -> bool {
    generations.get(&key.id) == Some(&key.generation)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyKey {
    Function(FunctionKey),
}

impl DependencyKey {
    pub const fn function(function: FunctionKey) -> Self {
        Self::Function(function)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyError {
    StaleDependency,
    VersionRegression,
}

#[derive(Clone, Debug)]
struct DependencyNode {
    version: u64,
    valid: bool,
    dependencies: BTreeSet<DependencyKey>,
}

#[derive(Clone, Debug, Default)]
pub struct DependencyGraph {
    nodes: BTreeMap<DependencyKey, DependencyNode>,
    reverse: BTreeMap<DependencyKey, BTreeSet<DependencyKey>>,
}

impl DependencyGraph {
    pub fn install(
        &mut self,
        key: DependencyKey,
        version: u64,
        dependencies: impl IntoIterator<Item = DependencyKey>,
    ) -> Result<(), DependencyError> {
        if self
            .nodes
            .get(&key)
            .is_some_and(|node| version < node.version)
        {
            return Err(DependencyError::VersionRegression);
        }
        let dependencies = dependencies.into_iter().collect::<BTreeSet<_>>();
        if dependencies
            .iter()
            .any(|dependency| !self.nodes.get(dependency).is_some_and(|node| node.valid))
        {
            return Err(DependencyError::StaleDependency);
        }
        if let Some(old) = self.nodes.get(&key) {
            for dependency in &old.dependencies {
                if let Some(dependents) = self.reverse.get_mut(dependency) {
                    dependents.remove(&key);
                }
            }
        }
        for dependency in &dependencies {
            self.reverse.entry(*dependency).or_default().insert(key);
        }
        self.nodes.insert(
            key,
            DependencyNode {
                version,
                valid: true,
                dependencies,
            },
        );
        Ok(())
    }

    pub fn validate_install(
        &self,
        key: DependencyKey,
        version: u64,
        dependencies: &[(DependencyKey, u64)],
    ) -> bool {
        self.nodes
            .get(&key)
            .is_some_and(|node| node.valid && node.version == version)
            && dependencies.iter().all(|(dependency, expected)| {
                self.nodes
                    .get(dependency)
                    .is_some_and(|node| node.valid && node.version == *expected)
            })
    }

    pub fn invalidate(&mut self, root: DependencyKey) -> Vec<DependencyKey> {
        if !self.nodes.contains_key(&root) && !self.reverse.contains_key(&root) {
            return Vec::new();
        }
        let mut queue = VecDeque::from([root]);
        let mut invalidated = BTreeSet::new();
        while let Some(key) = queue.pop_front() {
            if !invalidated.insert(key) {
                continue;
            }
            if let Some(node) = self.nodes.get_mut(&key) {
                node.valid = false;
            }
            if let Some(dependents) = self.reverse.get(&key) {
                queue.extend(dependents.iter().copied());
            }
        }
        invalidated.into_iter().collect()
    }
}
