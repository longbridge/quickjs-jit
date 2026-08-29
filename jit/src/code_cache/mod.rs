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
    charge_bytes: usize,
}

impl CachedArtifact {
    fn new(
        artifact: CompiledArtifact,
        last_used: u64,
        deopt_target: Option<DeoptPin>,
        charge_bytes: usize,
    ) -> Self {
        Self {
            artifact,
            execution_pins: AtomicUsize::new(0),
            deopt_references: AtomicUsize::new(0),
            last_used: AtomicU64::new(last_used),
            invalidated: AtomicBool::new(false),
            _deopt_target: deopt_target,
            charge_bytes,
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
    ArtifactTooLarge,
    ChargeOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheInsert {
    evicted: Box<[ArtifactKey]>,
}

impl CacheInsert {
    pub fn evicted(&self) -> Option<ArtifactKey> {
        self.evicted.first().copied()
    }

    pub fn evictions(&self) -> &[ArtifactKey] {
        &self.evicted
    }
}

#[derive(Debug)]
pub struct CodeCache {
    max_bytes: usize,
    charged_bytes: usize,
    clock: u64,
    artifacts: BTreeMap<ArtifactKey, Arc<CachedArtifact>>,
    reclaim_needed: Arc<AtomicBool>,
}

impl CodeCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            charged_bytes: 0,
            clock: 0,
            artifacts: BTreeMap::new(),
            reclaim_needed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    pub fn insert(&mut self, artifact: CompiledArtifact) -> Result<CacheInsert, CacheError> {
        if self.max_bytes == 0 {
            return Err(CacheError::CapacityZero);
        }
        let charge_bytes = artifact.charge_bytes().ok_or(CacheError::ChargeOverflow)?;
        if charge_bytes > self.max_bytes {
            return Err(CacheError::ArtifactTooLarge);
        }
        self.collect_invalidated();
        let key = artifact.key();
        let deopt_target = if key.tier == Tier::Optimizing {
            let baseline = self
                .artifacts
                .get(&key.with_tier(Tier::Baseline))
                .ok_or(CacheError::MissingDeoptTarget)?;
            Some(DeoptPin::new(
                Arc::clone(baseline),
                Arc::clone(&self.reclaim_needed),
            ))
        } else {
            None
        };
        let existing_charge = if let Some(existing) = self.artifacts.get(&key) {
            if !existing.is_evictable() {
                return Err(CacheError::ArtifactPinned);
            }
            existing.charge_bytes
        } else {
            0
        };
        let retained_bytes = self
            .charged_bytes
            .checked_sub(existing_charge)
            .ok_or(CacheError::ChargeOverflow)?;
        let desired_bytes = retained_bytes
            .checked_add(charge_bytes)
            .ok_or(CacheError::ChargeOverflow)?;
        let needed_bytes = desired_bytes.saturating_sub(self.max_bytes);
        let mut selected = Vec::new();
        let mut freed_bytes = 0usize;
        for (candidate, candidate_bytes) in evict::candidates(&self.artifacts, key) {
            if freed_bytes >= needed_bytes {
                break;
            }
            freed_bytes = freed_bytes
                .checked_add(candidate_bytes)
                .ok_or(CacheError::ChargeOverflow)?;
            selected.push(candidate);
        }
        if freed_bytes < needed_bytes {
            return Err(CacheError::AllArtifactsPinned);
        }
        if existing_charge != 0 || self.artifacts.contains_key(&key) {
            self.remove(key);
        }
        for candidate in &selected {
            self.remove(*candidate);
        }
        let tick = self.next_tick();
        self.charged_bytes = self
            .charged_bytes
            .checked_add(charge_bytes)
            .ok_or(CacheError::ChargeOverflow)?;
        self.artifacts.insert(
            key,
            Arc::new(CachedArtifact::new(
                artifact,
                tick,
                deopt_target,
                charge_bytes,
            )),
        );
        Ok(CacheInsert {
            evicted: selected.into_boxed_slice(),
        })
    }

    fn remove(&mut self, key: ArtifactKey) -> Option<Arc<CachedArtifact>> {
        let artifact = self.artifacts.remove(&key)?;
        self.charged_bytes = self.charged_bytes.saturating_sub(artifact.charge_bytes);
        Some(artifact)
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
        Some(ExecutionPin {
            artifact,
            reclaim_needed: Arc::clone(&self.reclaim_needed),
        })
    }

    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub const fn charged_bytes(&self) -> usize {
        self.charged_bytes
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
            self.remove(candidate);
            removed = removed.saturating_add(1);
        }
        removed
    }

    /// Reclaims invalidated artifacts after a pin release has made progress possible.
    pub fn poll_reclamation(&mut self) -> usize {
        if !self.reclaim_needed.swap(false, Ordering::AcqRel) {
            return 0;
        }
        self.collect_invalidated()
    }
}

#[derive(Debug)]
struct DeoptPin {
    target: Arc<CachedArtifact>,
    reclaim_needed: Arc<AtomicBool>,
}

impl DeoptPin {
    fn new(target: Arc<CachedArtifact>, reclaim_needed: Arc<AtomicBool>) -> Self {
        target.deopt_references.fetch_add(1, Ordering::AcqRel);
        Self {
            target,
            reclaim_needed,
        }
    }
}

impl Drop for DeoptPin {
    fn drop(&mut self) {
        let previous = self.target.deopt_references.fetch_sub(1, Ordering::AcqRel);
        if previous == 1 && self.target.invalidated.load(Ordering::Acquire) {
            self.reclaim_needed.store(true, Ordering::Release);
        }
    }
}

#[derive(Debug)]
pub struct ExecutionPin {
    artifact: Arc<CachedArtifact>,
    reclaim_needed: Arc<AtomicBool>,
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
        let previous = self.artifact.execution_pins.fetch_sub(1, Ordering::AcqRel);
        if previous == 1 && self.artifact.invalidated.load(Ordering::Acquire) {
            self.reclaim_needed.store(true, Ordering::Release);
        }
    }
}
