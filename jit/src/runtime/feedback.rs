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
    BigInt,
    Symbol,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryFeedbackFlags(u32);

impl BinaryFeedbackFlags {
    pub const NONE: Self = Self(0);
    pub const OVERFLOW: Self = Self(1 << 0);
    pub const NEGATIVE_ZERO: Self = Self(1 << 1);
    pub const NAN: Self = Self(1 << 2);

    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOrAssign for BinaryFeedbackFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
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
    calls: Box<[CallFeedbackSnapshot]>,
    binaries: Box<[BinaryFeedbackSnapshot]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryFeedbackSnapshot {
    function: FunctionKey,
    pc: u32,
    state: FeedbackState,
    lhs: Box<[ObservedType]>,
    rhs: Box<[ObservedType]>,
    result: Box<[ObservedType]>,
    flags: BinaryFeedbackFlags,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallFeedbackSnapshot {
    function: FunctionKey,
    state: FeedbackState,
    arguments: Box<[Box<[ObservedType]>]>,
    stable_types: Option<Box<[ObservedType]>>,
}

impl CallFeedbackSnapshot {
    pub const fn state(&self) -> FeedbackState {
        self.state
    }
    pub fn argc(&self) -> usize {
        self.arguments.len()
    }
    pub fn argument(&self, index: usize) -> &[ObservedType] {
        self.arguments.get(index).map_or(&[], Box::as_ref)
    }
}

impl BinaryFeedbackSnapshot {
    pub const fn state(&self) -> FeedbackState {
        self.state
    }
    pub fn lhs(&self) -> &[ObservedType] {
        &self.lhs
    }
    pub fn rhs(&self) -> &[ObservedType] {
        &self.rhs
    }
    pub fn result(&self) -> &[ObservedType] {
        &self.result
    }
    pub const fn flags(&self) -> BinaryFeedbackFlags {
        self.flags
    }
}

impl FeedbackSnapshot {
    pub fn empty(epoch: u64) -> Self {
        Self {
            epoch,
            entries: Box::new([]),
            calls: Box::new([]),
            binaries: Box::new([]),
        }
    }
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn entries(&self) -> &[FeedbackSnapshotEntry] {
        &self.entries
    }

    pub fn function(&self) -> Option<FunctionKey> {
        let function = self
            .entries
            .first()
            .map(|entry| entry.function)
            .or_else(|| self.calls.first().map(|entry| entry.function))
            .or_else(|| self.binaries.first().map(|entry| entry.function))?;
        let same = self.entries.iter().all(|entry| entry.function == function)
            && self.calls.iter().all(|entry| entry.function == function)
            && self.binaries.iter().all(|entry| entry.function == function);
        same.then_some(function)
    }

    pub fn call_argument_types(&self, function: FunctionKey) -> Option<&[ObservedType]> {
        self.call_at(function)?.stable_types.as_deref()
    }

    pub fn call_at(&self, function: FunctionKey) -> Option<&CallFeedbackSnapshot> {
        self.calls.iter().find(|entry| entry.function == function)
    }

    pub fn stable_return_at(&self, function: FunctionKey, pc: u32) -> Option<ObservedType> {
        self.entries.iter().find_map(|entry| {
            (entry.function == function
                && entry.pc == pc
                && entry.kind == FeedbackKind::Exit
                && entry.state == FeedbackState::Monomorphic
                && entry.observations.len() == 1)
                .then_some(entry.observations[0])
        })
    }

    pub fn binary_at(&self, function: FunctionKey, pc: u32) -> Option<&BinaryFeedbackSnapshot> {
        self.binaries
            .iter()
            .find(|entry| entry.function == function && entry.pc == pc)
    }

    /// Tier 2 may only consume feedback belonging to the exact function
    /// generation and frozen at a nonzero runtime epoch.
    pub fn has_stable_value_for(&self, function: FunctionKey) -> bool {
        self.epoch != 0
            && self.entries.iter().any(|entry| {
                entry.function == function
                    && entry.kind == FeedbackKind::Value
                    && entry.state == FeedbackState::Monomorphic
                    && entry.observations.len() == 1
            })
    }

    pub fn contains_stable_observation(
        &self,
        function: FunctionKey,
        pc: u32,
        observation: ObservedType,
    ) -> bool {
        self.entries.iter().any(|entry| {
            entry.function == function
                && entry.pc == pc
                && entry.kind == FeedbackKind::Exit
                && entry.state == FeedbackState::Monomorphic
                && entry.observations.as_ref() == [observation]
        })
    }

    pub fn stable_observation_at(&self, function: FunctionKey, pc: u32) -> Option<ObservedType> {
        self.entries.iter().find_map(|entry| {
            (entry.function == function
                && entry.pc == pc
                && entry.kind == FeedbackKind::Value
                && entry.state == FeedbackState::Monomorphic
                && entry.observations.len() == 1)
                .then_some(entry.observations[0])
        })
    }
}

#[derive(Debug)]
pub struct FeedbackTable {
    capacity: usize,
    diversity_limit: usize,
    dropped: u64,
    entries: BTreeMap<FeedbackKey, Entry>,
    calls: BTreeMap<FunctionKey, Vec<Entry>>,
    binaries: BTreeMap<(FunctionKey, u32), BinaryFeedbackEntry>,
}

#[derive(Debug)]
struct BinaryFeedbackEntry {
    lhs: Entry,
    rhs: Entry,
    result: Entry,
    flags: BinaryFeedbackFlags,
}

fn empty_entry(capacity: usize) -> Entry {
    Entry {
        observations: Vec::with_capacity(capacity),
        megamorphic: false,
    }
}

impl FeedbackTable {
    pub fn new(capacity: usize, diversity_limit: usize) -> Self {
        Self {
            capacity,
            diversity_limit: diversity_limit.max(1),
            dropped: 0,
            entries: BTreeMap::new(),
            calls: BTreeMap::new(),
            binaries: BTreeMap::new(),
        }
    }

    pub fn observe_call(&mut self, function: FunctionKey, arguments: &[ObservedType]) {
        let slots = self.calls.entry(function).or_default();
        if slots.len() < arguments.len() {
            slots.resize_with(arguments.len(), || empty_entry(self.diversity_limit));
        }
        for (slot, observation) in slots.iter_mut().zip(arguments.iter().copied()) {
            slot.observe(observation, self.diversity_limit);
        }
    }

    pub fn observe_return(&mut self, function: FunctionKey, pc: u32, result: ObservedType) {
        self.observe_type(function, pc, FeedbackKind::Exit, result);
    }

    pub fn observe_binary(
        &mut self,
        function: FunctionKey,
        pc: u32,
        lhs: ObservedType,
        rhs: ObservedType,
        result: ObservedType,
        flags: BinaryFeedbackFlags,
    ) -> FeedbackState {
        let entry = self
            .binaries
            .entry((function, pc))
            .or_insert_with(|| BinaryFeedbackEntry {
                lhs: empty_entry(self.diversity_limit),
                rhs: empty_entry(self.diversity_limit),
                result: empty_entry(self.diversity_limit),
                flags: BinaryFeedbackFlags::NONE,
            });
        let states = [
            entry.lhs.observe(lhs, self.diversity_limit),
            entry.rhs.observe(rhs, self.diversity_limit),
            entry.result.observe(result, self.diversity_limit),
        ];
        entry.flags |= flags;
        if states.contains(&FeedbackState::Megamorphic) {
            FeedbackState::Megamorphic
        } else if states.contains(&FeedbackState::Polymorphic) {
            FeedbackState::Polymorphic
        } else {
            FeedbackState::Monomorphic
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
        let calls = self
            .calls
            .iter()
            .map(|(function, slots)| {
                let state = if slots.iter().any(|slot| slot.megamorphic) {
                    FeedbackState::Megamorphic
                } else if slots.iter().any(|slot| slot.observations.len() != 1) {
                    FeedbackState::Polymorphic
                } else {
                    FeedbackState::Monomorphic
                };
                let stable_types = (state == FeedbackState::Monomorphic).then(|| {
                    slots
                        .iter()
                        .map(|slot| slot.observations[0])
                        .collect::<Vec<_>>()
                        .into_boxed_slice()
                });
                CallFeedbackSnapshot {
                    function: *function,
                    state,
                    arguments: slots
                        .iter()
                        .map(|slot| slot.observations.clone().into_boxed_slice())
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                    stable_types,
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let state = |entries: [&Entry; 3]| {
            if entries.iter().any(|entry| entry.megamorphic) {
                FeedbackState::Megamorphic
            } else if entries.iter().any(|entry| entry.observations.len() > 1) {
                FeedbackState::Polymorphic
            } else {
                FeedbackState::Monomorphic
            }
        };
        let binaries = self
            .binaries
            .iter()
            .map(|((function, pc), entry)| BinaryFeedbackSnapshot {
                function: *function,
                pc: *pc,
                state: state([&entry.lhs, &entry.rhs, &entry.result]),
                lhs: entry.lhs.observations.clone().into_boxed_slice(),
                rhs: entry.rhs.observations.clone().into_boxed_slice(),
                result: entry.result.observations.clone().into_boxed_slice(),
                flags: entry.flags,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        FeedbackSnapshot {
            epoch,
            entries,
            calls,
            binaries,
        }
    }
}
