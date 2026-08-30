use std::collections::BTreeMap;

use super::FunctionKey;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FeedbackKind {
    Value,
    CallTarget,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ObservedType {
    Int32,
    Float64,
    Bool,
    Null,
    Undefined,
    String,
    Object,
    Function(FunctionKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackState {
    Monomorphic,
    Polymorphic,
    Megamorphic,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct FeedbackKey {
    function: FunctionKey,
    pc: u32,
    kind: FeedbackKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    observations: Vec<ObservedType>,
    megamorphic: bool,
}

impl Entry {
    fn observe(&mut self, observation: ObservedType, diversity_limit: usize) -> FeedbackState {
        if self.megamorphic {
            return FeedbackState::Megamorphic;
        }
        if !self.observations.contains(&observation) {
            if self.observations.len() >= diversity_limit {
                self.megamorphic = true;
                self.observations.clear();
                return FeedbackState::Megamorphic;
            }
            self.observations.push(observation);
        }
        if self.observations.len() == 1 {
            FeedbackState::Monomorphic
        } else {
            FeedbackState::Polymorphic
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedbackSnapshotEntry {
    function: FunctionKey,
    pc: u32,
    kind: FeedbackKind,
    state: FeedbackState,
    observations: Box<[ObservedType]>,
}

impl FeedbackSnapshotEntry {
    pub const fn function(&self) -> FunctionKey {
        self.function
    }
    pub const fn pc(&self) -> u32 {
        self.pc
    }
    pub const fn kind(&self) -> FeedbackKind {
        self.kind
    }
    pub const fn state(&self) -> FeedbackState {
        self.state
    }
    pub fn observations(&self) -> &[ObservedType] {
        &self.observations
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedbackSnapshot {
    epoch: u64,
    entries: Box<[FeedbackSnapshotEntry]>,
}

impl FeedbackSnapshot {
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn entries(&self) -> &[FeedbackSnapshotEntry] {
        &self.entries
    }
}

#[derive(Debug)]
pub struct FeedbackTable {
    capacity: usize,
    diversity_limit: usize,
    dropped: u64,
    entries: BTreeMap<FeedbackKey, Entry>,
}

impl FeedbackTable {
    pub fn new(capacity: usize, diversity_limit: usize) -> Self {
        Self {
            capacity,
            diversity_limit: diversity_limit.max(1),
            dropped: 0,
            entries: BTreeMap::new(),
        }
    }

    pub fn observe_type(
        &mut self,
        function: FunctionKey,
        pc: u32,
        kind: FeedbackKind,
        observation: ObservedType,
    ) -> FeedbackState {
        let key = FeedbackKey { function, pc, kind };
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            self.dropped = self.dropped.saturating_add(1);
            return FeedbackState::Megamorphic;
        }
        self.entries
            .entry(key)
            .or_insert_with(|| Entry {
                observations: Vec::with_capacity(self.diversity_limit),
                megamorphic: false,
            })
            .observe(observation, self.diversity_limit)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub const fn dropped_observations(&self) -> u64 {
        self.dropped
    }

    pub fn snapshot(&self, epoch: u64) -> FeedbackSnapshot {
        let entries = self
            .entries
            .iter()
            .map(|(key, entry)| FeedbackSnapshotEntry {
                function: key.function,
                pc: key.pc,
                kind: key.kind,
                state: if entry.megamorphic {
                    FeedbackState::Megamorphic
                } else if entry.observations.len() == 1 {
                    FeedbackState::Monomorphic
                } else {
                    FeedbackState::Polymorphic
                },
                observations: entry.observations.clone().into_boxed_slice(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        FeedbackSnapshot { epoch, entries }
    }
}
