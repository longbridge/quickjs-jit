//! Owned compiled artifacts and their runtime cache.

mod artifact;
mod evict;

pub use artifact::{
    ArtifactDependency, ArtifactKey, BenefitSnapshot, CodeAllocation, CompiledArtifact, FrameState,
    Relocation, StackMap,
};

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

use crate::runtime::{FunctionKey, Tier};

#[derive(Debug)]
pub(super) struct CachedArtifact {
    artifact: CompiledArtifact,
    execution_pins: AtomicUsize,
    deopt_references: AtomicUsize,
    last_used: AtomicU64,
    invalidated: AtomicBool,
    _deopt_target: Option<DeoptPin>,
}

impl CachedArtifact {
    fn new(artifact: CompiledArtifact, last_used: u64, deopt_target: Option<DeoptPin>) -> Self {
        Self {
            artifact,
            execution_pins: AtomicUsize::new(0),
            deopt_references: AtomicUsize::new(0),
            last_used: AtomicU64::new(last_used),
            invalidated: AtomicBool::new(false),
            _deopt_target: deopt_target,
        }
    }

    pub(super) fn is_evictable(&self) -> bool {
        self.execution_pins.load(Ordering::Acquire) == 0
            && self.deopt_references.load(Ordering::Acquire) == 0
    }

    pub(super) fn eviction_order(&self) -> (u64, u64, ArtifactKey) {
        (
            self.artifact.benefit_score(),
            self.last_used.load(Ordering::Acquire),
            self.artifact.key(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    CapacityZero,
    AllArtifactsPinned,
    ArtifactPinned,
    MissingArtifact,
    MissingDeoptTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheInsert {
    evicted: Option<ArtifactKey>,
}

impl CacheInsert {
    pub const fn evicted(self) -> Option<ArtifactKey> {
        self.evicted
    }
}

#[derive(Debug)]
pub struct CodeCache {
    capacity: usize,
    clock: u64,
    artifacts: BTreeMap<ArtifactKey, Arc<CachedArtifact>>,
}

impl CodeCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            clock: 0,
            artifacts: BTreeMap::new(),
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    pub fn insert(&mut self, artifact: CompiledArtifact) -> Result<CacheInsert, CacheError> {
        if self.capacity == 0 {
            return Err(CacheError::CapacityZero);
        }
        self.collect_invalidated();
        let key = artifact.key();
        let deopt_target = if key.tier == Tier::Optimizing {
            let baseline = self
                .artifacts
                .get(&key.with_tier(Tier::Baseline))
                .ok_or(CacheError::MissingDeoptTarget)?;
            Some(DeoptPin::new(Arc::clone(baseline)))
        } else {
            None
        };
        if let Some(existing) = self.artifacts.get(&key) {
            if !existing.is_evictable() {
                return Err(CacheError::ArtifactPinned);
            }
            self.artifacts.remove(&key);
        }
        let evicted = if self.artifacts.len() >= self.capacity {
            let candidate =
                evict::candidate(&self.artifacts).ok_or(CacheError::AllArtifactsPinned)?;
            self.artifacts.remove(&candidate);
            Some(candidate)
        } else {
            None
        };
        let tick = self.next_tick();
        self.artifacts.insert(
            key,
            Arc::new(CachedArtifact::new(artifact, tick, deopt_target)),
        );
        Ok(CacheInsert { evicted })
    }

    pub fn contains(&self, key: ArtifactKey) -> bool {
        self.artifacts.contains_key(&key)
    }

    pub fn touch(&mut self, key: ArtifactKey) -> bool {
        let tick = self.next_tick();
        let Some(artifact) = self.artifacts.get(&key) else {
            return false;
        };
        artifact.last_used.store(tick, Ordering::Release);
        true
    }

    pub fn record_benefit(&mut self, key: ArtifactKey, benefit: u64) -> Result<(), CacheError> {
        let tick = self.next_tick();
        let artifact = self
            .artifacts
            .get(&key)
            .ok_or(CacheError::MissingArtifact)?;
        artifact.artifact.record_benefit(benefit);
        artifact.last_used.store(tick, Ordering::Release);
        Ok(())
    }

    pub fn pin(&mut self, key: ArtifactKey) -> Option<ExecutionPin> {
        let tick = self.next_tick();
        let artifact = Arc::clone(self.artifacts.get(&key)?);
        if artifact.invalidated.load(Ordering::Acquire) {
            return None;
        }
        artifact.execution_pins.fetch_add(1, Ordering::AcqRel);
        artifact.last_used.store(tick, Ordering::Release);
        Some(ExecutionPin { artifact })
    }

    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub fn deopt_references(&self, key: ArtifactKey) -> Option<usize> {
        self.artifacts
            .get(&key)
            .map(|artifact| artifact.deopt_references.load(Ordering::Acquire))
    }

    pub fn invalidate(&mut self, function: FunctionKey) -> usize {
        for (key, artifact) in &self.artifacts {
            if key.function_id == function.id && key.generation == function.generation {
                artifact.invalidated.store(true, Ordering::Release);
            }
        }
        self.collect_invalidated()
    }

    pub fn collect_invalidated(&mut self) -> usize {
        let mut removed = 0usize;
        loop {
            let candidate = self
                .artifacts
                .iter()
                .filter(|(_, artifact)| {
                    artifact.invalidated.load(Ordering::Acquire) && artifact.is_evictable()
                })
                .min_by_key(|(key, _)| {
                    let tier_order = match key.tier {
                        Tier::Optimizing => 0,
                        Tier::Baseline => 1,
                    };
                    (tier_order, **key)
                })
                .map(|(key, _)| *key);
            let Some(candidate) = candidate else {
                break;
            };
            self.artifacts.remove(&candidate);
            removed = removed.saturating_add(1);
        }
        removed
    }
}

#[derive(Debug)]
struct DeoptPin {
    target: Arc<CachedArtifact>,
}

impl DeoptPin {
    fn new(target: Arc<CachedArtifact>) -> Self {
        target.deopt_references.fetch_add(1, Ordering::AcqRel);
        Self { target }
    }
}

impl Drop for DeoptPin {
    fn drop(&mut self) {
        self.target.deopt_references.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub struct ExecutionPin {
    artifact: Arc<CachedArtifact>,
}

impl ExecutionPin {
    pub fn key(&self) -> ArtifactKey {
        self.artifact.artifact.key()
    }

    pub fn artifact(&self) -> &CompiledArtifact {
        &self.artifact.artifact
    }
}

impl Drop for ExecutionPin {
    fn drop(&mut self) {
        self.artifact.execution_pins.fetch_sub(1, Ordering::AcqRel);
    }
}
