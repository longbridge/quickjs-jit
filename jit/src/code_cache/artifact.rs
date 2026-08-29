use std::sync::atomic::{AtomicU64, Ordering};

use crate::runtime::{FunctionKey, Tier};

/// Full compatibility identity for one compiled artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactKey {
    pub runtime_id: u64,
    pub function_id: u64,
    pub generation: u64,
    pub tier: Tier,
    pub target_isa: u64,
    pub cpu_features: u64,
    pub abi_fingerprint: u64,
    pub source_revision: u64,
    pub opcode_fingerprint: u64,
    pub config_fingerprint: u64,
}

impl ArtifactKey {
    pub const fn with_tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct CodeAllocation {
    bytes: Box<[u8]>,
}

impl CodeAllocation {
    pub fn inert(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into_boxed_slice(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn is_executable(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Relocation {
    pub offset: u32,
    pub target: u64,
    pub addend: i64,
}

impl Relocation {
    pub const fn new(offset: u32, target: u64, addend: i64) -> Self {
        Self {
            offset,
            target,
            addend,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackMap {
    pub code_offset: u32,
    pub live_slots: Box<[u16]>,
}

impl StackMap {
    pub fn new(code_offset: u32, live_slots: Vec<u16>) -> Self {
        Self {
            code_offset,
            live_slots: live_slots.into_boxed_slice(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameState {
    pub code_offset: u32,
    pub bytecode_pc: u32,
    pub slots: Box<[u16]>,
}

impl FrameState {
    pub fn new(code_offset: u32, bytecode_pc: u32, slots: Vec<u16>) -> Self {
        Self {
            code_offset,
            bytecode_pc,
            slots: slots.into_boxed_slice(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactDependency {
    pub function: FunctionKey,
}

impl ArtifactDependency {
    pub const fn new(function: FunctionKey) -> Self {
        Self { function }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BenefitSnapshot {
    pub executions: u64,
    pub score: u64,
}

#[derive(Debug, Default)]
struct BenefitCounters {
    executions: AtomicU64,
    score: AtomicU64,
}

impl BenefitCounters {
    fn record(&self, score: u64) {
        let _ = self
            .executions
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(1))
            });
        let _ = self
            .score
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(score))
            });
    }

    fn snapshot(&self) -> BenefitSnapshot {
        BenefitSnapshot {
            executions: self.executions.load(Ordering::Acquire),
            score: self.score.load(Ordering::Acquire),
        }
    }
}

/// Compiler output. Task 5 allocations contain inert owned bytes only.
#[derive(Debug)]
pub struct CompiledArtifact {
    key: ArtifactKey,
    code: CodeAllocation,
    relocations: Box<[Relocation]>,
    stack_maps: Box<[StackMap]>,
    frame_states: Box<[FrameState]>,
    dependencies: Box<[ArtifactDependency]>,
    benefit: BenefitCounters,
    #[cfg(any(test, feature = "test-support"))]
    fake: bool,
}

impl CompiledArtifact {
    pub fn empty(key: ArtifactKey) -> Self {
        Self::from_parts(
            key,
            CodeAllocation::inert(Vec::new()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    pub fn from_parts(
        key: ArtifactKey,
        code: CodeAllocation,
        relocations: Vec<Relocation>,
        stack_maps: Vec<StackMap>,
        frame_states: Vec<FrameState>,
        dependencies: Vec<ArtifactDependency>,
    ) -> Self {
        Self {
            key,
            code,
            relocations: relocations.into_boxed_slice(),
            stack_maps: stack_maps.into_boxed_slice(),
            frame_states: frame_states.into_boxed_slice(),
            dependencies: dependencies.into_boxed_slice(),
            benefit: BenefitCounters::default(),
            #[cfg(any(test, feature = "test-support"))]
            fake: false,
        }
    }

    pub const fn key(&self) -> ArtifactKey {
        self.key
    }

    pub const fn code(&self) -> &CodeAllocation {
        &self.code
    }

    pub fn relocations(&self) -> &[Relocation] {
        &self.relocations
    }

    pub fn stack_maps(&self) -> &[StackMap] {
        &self.stack_maps
    }

    pub fn frame_states(&self) -> &[FrameState] {
        &self.frame_states
    }

    pub fn dependencies(&self) -> &[ArtifactDependency] {
        &self.dependencies
    }

    pub fn benefit(&self) -> BenefitSnapshot {
        self.benefit.snapshot()
    }

    pub fn charge_bytes(&self) -> Option<usize> {
        fn add_slice<T>(total: usize, len: usize) -> Option<usize> {
            total.checked_add(len.checked_mul(core::mem::size_of::<T>())?)
        }

        let mut total = self.code.bytes().len();
        total = add_slice::<Relocation>(total, self.relocations.len())?;
        total = add_slice::<StackMap>(total, self.stack_maps.len())?;
        for map in &self.stack_maps {
            total = add_slice::<u16>(total, map.live_slots.len())?;
        }
        total = add_slice::<FrameState>(total, self.frame_states.len())?;
        for state in &self.frame_states {
            total = add_slice::<u16>(total, state.slots.len())?;
        }
        add_slice::<ArtifactDependency>(total, self.dependencies.len())
    }

    pub(crate) fn record_benefit(&self, score: u64) {
        self.benefit.record(score);
    }

    pub(crate) fn benefit_score(&self) -> u64 {
        self.benefit.score.load(Ordering::Acquire)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fake(tier: Tier) -> Self {
        Self {
            key: ArtifactKey {
                runtime_id: 0,
                function_id: 0,
                generation: 0,
                tier,
                target_isa: 0,
                cpu_features: 0,
                abi_fingerprint: 0,
                source_revision: 0,
                opcode_fingerprint: 0,
                config_fingerprint: 0,
            },
            code: CodeAllocation::inert(Vec::new()),
            relocations: Box::new([]),
            stack_maps: Box::new([]),
            frame_states: Box::new([]),
            dependencies: Box::new([]),
            benefit: BenefitCounters::default(),
            fake: true,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn bind_fake(mut self, key: ArtifactKey) -> Self {
        if self.fake && self.key.tier == key.tier {
            self.key = key;
            self.fake = false;
        }
        self
    }
}
