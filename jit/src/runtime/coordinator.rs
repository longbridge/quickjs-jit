use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
};

use crate::{
    bytecode::VerifiedFunction,
    code_cache::{ArtifactKey, CodeCache, CompiledArtifact, ExecutionPin},
    compiler::CompileFailure,
    JitMetrics,
};

use super::{install, invalidate, DependencyGraph, DependencyKey};

pub fn compile_and_send<C: crate::compiler::Compiler + ?Sized>(
    compiler: &C,
    request: CompileRequest,
    sender: &CompletionSender,
) -> Result<(), Box<CompileCompletion>> {
    let key = request.key();
    let requested_tier = request.tier();
    let artifact_key = request.artifact_key();
    let attempt_id = request.attempt_id();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compiler.compile(request)))
            .unwrap_or(Err(CompileFailure::CompilerPanicked));
    sender.send(CompileCompletion {
        key,
        requested_tier,
        artifact_key,
        attempt_id,
        result,
    })
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FunctionKey {
    pub id: u64,
    pub generation: u64,
}

impl FunctionKey {
    pub const fn new(id: u64, generation: u64) -> Self {
        Self { id, generation }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Tier {
    Baseline,
    Optimizing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactEnvironment {
    pub runtime_id: u64,
    pub target_isa: u64,
    pub cpu_features: u64,
    pub abi_fingerprint: u64,
    pub config_fingerprint: u64,
}

impl Default for ArtifactEnvironment {
    fn default() -> Self {
        Self {
            runtime_id: 1,
            target_isa: 0,
            cpu_features: 0,
            abi_fingerprint: 0,
            config_fingerprint: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileState {
    Cold,
    Queued(Tier),
    Compiling(Tier),
    Ready(Tier),
    Installed(Tier),
    Backoff { attempts: u8, retry_after: u64 },
    Blacklisted,
    Retired,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttemptId(u64);

impl AttemptId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct CompileRequest {
    key: FunctionKey,
    tier: Tier,
    snapshot: VerifiedFunction,
    artifact_key: ArtifactKey,
    attempt_id: AttemptId,
    feedback_epoch: u64,
}

impl CompileRequest {
    pub const fn key(&self) -> FunctionKey {
        self.key
    }

    pub const fn tier(&self) -> Tier {
        self.tier
    }

    pub const fn artifact_key(&self) -> ArtifactKey {
        self.artifact_key
    }

    pub const fn snapshot(&self) -> &VerifiedFunction {
        &self.snapshot
    }

    pub const fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }

    pub const fn feedback_epoch(&self) -> u64 {
        self.feedback_epoch
    }
}

#[derive(Debug)]
pub struct CompileCompletion {
    pub key: FunctionKey,
    pub requested_tier: Tier,
    pub artifact_key: ArtifactKey,
    pub attempt_id: AttemptId,
    pub result: Result<CompiledArtifact, CompileFailure>,
}

#[derive(Debug)]
pub enum CompletionSendError {
    Full(Box<CompileCompletion>),
    Closed(Box<CompileCompletion>),
}

#[derive(Clone, Debug)]
pub struct CompletionSender {
    sender: Option<SyncSender<CompileCompletion>>,
    signals: Arc<CompletionQueueSignals>,
}

#[derive(Debug, Default)]
struct CompletionQueueSignals {
    pending: AtomicUsize,
    saturated: AtomicU64,
}

impl CompletionQueueSignals {
    fn increment_pending(&self) {
        let _ = self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                Some(pending.saturating_add(1))
            });
    }

    fn decrement_pending(&self) {
        let _ = self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                Some(pending.saturating_sub(1))
            });
    }

    fn record_saturation(&self) {
        let _ = self
            .saturated
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_add(1))
            });
    }
}

impl CompletionSender {
    pub fn try_send(&self, completion: CompileCompletion) -> Result<(), CompletionSendError> {
        let Some(sender) = &self.sender else {
            return Err(CompletionSendError::Closed(Box::new(completion)));
        };
        self.signals.increment_pending();
        sender.try_send(completion).map_err(|error| match error {
            TrySendError::Full(completion) => {
                self.signals.decrement_pending();
                self.signals.record_saturation();
                CompletionSendError::Full(Box::new(completion))
            }
            TrySendError::Disconnected(completion) => {
                self.signals.decrement_pending();
                CompletionSendError::Closed(Box::new(completion))
            }
        })
    }

    pub fn send(&self, completion: CompileCompletion) -> Result<(), Box<CompileCompletion>> {
        let Some(sender) = &self.sender else {
            return Err(Box::new(completion));
        };
        self.signals.increment_pending();
        sender.send(completion).map_err(|error| {
            self.signals.decrement_pending();
            Box::new(error.0)
        })
    }
}

pub const DEFAULT_COMPLETION_DRAIN_BUDGET: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionDrain {
    drained: usize,
    reclaimed: usize,
    may_have_remaining: bool,
}

impl CompletionDrain {
    pub const fn drained(self) -> usize {
        self.drained
    }

    pub const fn reclaimed(self) -> usize {
        self.reclaimed
    }

    /// Conservatively reports whether completion or reclamation work may remain.
    pub const fn may_have_remaining(self) -> bool {
        self.may_have_remaining
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    Full,
    NotReady,
    Retired,
    Blacklisted,
    Shutdown,
    SnapshotIdentity,
    AttemptIdsExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InFlight {
    attempt_id: AttemptId,
    artifact_key: ArtifactKey,
    tier: Tier,
}

#[derive(Debug)]
struct TierRecord {
    state: CompileState,
    attempts: u8,
}

impl Default for TierRecord {
    fn default() -> Self {
        Self {
            state: CompileState::Cold,
            attempts: 0,
        }
    }
}

#[derive(Debug, Default)]
struct FunctionState {
    baseline: TierRecord,
    optimizing: TierRecord,
    published: Option<Tier>,
    retired: bool,
}

impl FunctionState {
    fn tier(&self, tier: Tier) -> &TierRecord {
        match tier {
            Tier::Baseline => &self.baseline,
            Tier::Optimizing => &self.optimizing,
        }
    }

    fn tier_mut(&mut self, tier: Tier) -> &mut TierRecord {
        match tier {
            Tier::Baseline => &mut self.baseline,
            Tier::Optimizing => &mut self.optimizing,
        }
    }

    fn active_state(&self) -> Option<CompileState> {
        [self.baseline.state, self.optimizing.state]
            .into_iter()
            .find(|state| {
                matches!(
                    state,
                    CompileState::Queued(_) | CompileState::Compiling(_) | CompileState::Ready(_)
                )
            })
    }

    fn visible_state(&self) -> CompileState {
        if self.retired {
            return CompileState::Retired;
        }
        if let Some(active) = self.active_state() {
            return active;
        }
        if let Some(published) = self.published {
            return CompileState::Installed(published);
        }
        [self.baseline.state, self.optimizing.state]
            .into_iter()
            .find(|state| {
                matches!(
                    state,
                    CompileState::Backoff { .. } | CompileState::Blacklisted
                )
            })
            .unwrap_or(CompileState::Cold)
    }
}

#[derive(Debug)]
pub struct Coordinator {
    max_queue_len: usize,
    max_attempts: u8,
    clock: u64,
    queue: VecDeque<CompileRequest>,
    functions: HashMap<FunctionKey, FunctionState>,
    current_generations: HashMap<u64, u64>,
    in_flight: HashMap<FunctionKey, InFlight>,
    next_attempt_id: u64,
    metrics: JitMetrics,
    completion_sender: Option<SyncSender<CompileCompletion>>,
    completion_receiver: Option<Receiver<CompileCompletion>>,
    completion_signals: Arc<CompletionQueueSignals>,
    shutdown: bool,
    cache: CodeCache,
    installed_keys: HashMap<(FunctionKey, Tier), ArtifactKey>,
    environment: ArtifactEnvironment,
    dependencies: DependencyGraph,
}

impl Coordinator {
    pub fn with_limits(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        max_code_bytes: usize,
    ) -> Self {
        Self::with_environment(
            max_queue_len,
            max_completion_len,
            max_attempts,
            max_code_bytes,
            ArtifactEnvironment::default(),
        )
    }

    pub fn with_environment(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        max_code_bytes: usize,
        environment: ArtifactEnvironment,
    ) -> Self {
        Self::with_cache(
            max_queue_len,
            max_completion_len,
            max_attempts,
            CodeCache::new(max_code_bytes),
            environment,
        )
    }

    pub fn with_environment_and_metadata_limit(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        max_code_bytes: usize,
        max_metadata_bytes: usize,
        environment: ArtifactEnvironment,
    ) -> Self {
        Self::with_cache(
            max_queue_len,
            max_completion_len,
            max_attempts,
            CodeCache::new_with_separate_limits(max_code_bytes, max_metadata_bytes),
            environment,
        )
    }

    fn with_cache(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        cache: CodeCache,
        environment: ArtifactEnvironment,
    ) -> Self {
        let (completion_sender, completion_receiver) = mpsc::sync_channel(max_completion_len);
        let completion_signals = Arc::new(CompletionQueueSignals::default());
        Self {
            max_queue_len,
            max_attempts,
            clock: 0,
            queue: VecDeque::new(),
            functions: HashMap::new(),
            current_generations: HashMap::new(),
            in_flight: HashMap::new(),
            next_attempt_id: 0,
            metrics: JitMetrics::disabled(),
            completion_sender: Some(completion_sender),
            completion_receiver: Some(completion_receiver),
            completion_signals,
            shutdown: false,
            cache,
            installed_keys: HashMap::new(),
            environment,
            dependencies: DependencyGraph::default(),
        }
    }

    pub fn queue(
        &mut self,
        key: FunctionKey,
        tier: Tier,
        snapshot: VerifiedFunction,
    ) -> Result<(), QueueError> {
        if self.shutdown {
            return Err(QueueError::Shutdown);
        }
        if self
            .current_generations
            .get(&key.id)
            .is_some_and(|generation| key.generation < *generation)
        {
            return Err(QueueError::Retired);
        }
        if self
            .functions
            .get(&key)
            .is_some_and(|function| function.retired)
        {
            return Err(QueueError::Retired);
        }
        if self
            .functions
            .get(&key)
            .and_then(FunctionState::active_state)
            .is_some()
        {
            return Err(QueueError::NotReady);
        }
        let installed_baseline = self.installed_keys.contains_key(&(key, Tier::Baseline));
        let installed_optimizing = self.installed_keys.contains_key(&(key, Tier::Optimizing));
        match tier {
            Tier::Baseline if installed_baseline || installed_optimizing => {
                return Err(QueueError::NotReady)
            }
            Tier::Optimizing if !installed_baseline || installed_optimizing => {
                return Err(QueueError::NotReady)
            }
            _ => {}
        }
        let tier_state = self
            .functions
            .get(&key)
            .map_or(CompileState::Cold, |function| function.tier(tier).state);
        match tier_state {
            CompileState::Cold => {}
            CompileState::Backoff { retry_after, .. } if self.clock >= retry_after => {}
            CompileState::Blacklisted => return Err(QueueError::Blacklisted),
            _ => return Err(QueueError::NotReady),
        }
        let source = snapshot.snapshot();
        if (source.function_id() != 0 && source.function_id() != key.id)
            || (source.generation() != 0 && source.generation() != key.generation)
        {
            return Err(QueueError::SnapshotIdentity);
        }
        let artifact_key = ArtifactKey {
            runtime_id: self.environment.runtime_id,
            function_id: key.id,
            generation: key.generation,
            tier,
            target_isa: self.environment.target_isa,
            cpu_features: self.environment.cpu_features,
            abi_fingerprint: self.environment.abi_fingerprint,
            source_revision: source.source_revision(),
            opcode_fingerprint: source.opcode_fingerprint(),
            config_fingerprint: self.environment.config_fingerprint,
        };
        if self
            .current_generations
            .get(&key.id)
            .is_none_or(|generation| key.generation > *generation)
        {
            self.retire_older_generations(key);
            self.current_generations.insert(key.id, key.generation);
        }
        if self.queue.len() >= self.max_queue_len {
            self.metrics.queue_saturated = self.metrics.queue_saturated.saturating_add(1);
            return Err(QueueError::Full);
        }
        let Some(next_attempt_id) = self.next_attempt_id.checked_add(1) else {
            return Err(QueueError::AttemptIdsExhausted);
        };
        self.next_attempt_id = next_attempt_id;
        self.queue.push_back(CompileRequest {
            key,
            tier,
            snapshot,
            artifact_key,
            attempt_id: AttemptId(next_attempt_id),
            feedback_epoch: self.clock,
        });
        let function = self.functions.entry(key).or_default();
        function.tier_mut(tier).state = CompileState::Queued(tier);
        self.metrics.queued = self.metrics.queued.saturating_add(1);
        Ok(())
    }

    pub fn begin_next(&mut self) -> Option<CompileRequest> {
        loop {
            let request = self.queue.pop_front()?;
            let Some(record) = self.functions.get_mut(&request.key) else {
                continue;
            };
            if record.tier(request.tier).state != CompileState::Queued(request.tier) {
                continue;
            }
            record.tier_mut(request.tier).state = CompileState::Compiling(request.tier);
            self.in_flight.insert(
                request.key,
                InFlight {
                    attempt_id: request.attempt_id,
                    artifact_key: request.artifact_key,
                    tier: request.tier,
                },
            );
            self.metrics.compiling = self.metrics.compiling.saturating_add(1);
            return Some(request);
        }
    }

    pub(super) fn rollback_dispatch(&mut self, request: CompileRequest) {
        self.rollback(request, true);
    }

    pub(super) fn rollback_resource_limit(&mut self, request: CompileRequest) {
        self.rollback(request, false);
    }

    fn rollback(&mut self, request: CompileRequest, saturated: bool) {
        if self.in_flight.get(&request.key).is_some_and(|flight| {
            flight.attempt_id == request.attempt_id && flight.tier == request.tier
        }) {
            self.in_flight.remove(&request.key);
            if let Some(record) = self.functions.get_mut(&request.key) {
                record.tier_mut(request.tier).state = CompileState::Queued(request.tier);
            }
            self.queue.push_front(request);
            if saturated {
                self.metrics.worker_queue_saturated =
                    self.metrics.worker_queue_saturated.saturating_add(1);
            }
        }
    }

    pub fn complete(&mut self, completion: CompileCompletion) {
        let Some(expected) = self.in_flight.get(&completion.key).copied() else {
            self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
            return;
        };
        if !invalidate::is_current_generation(&self.current_generations, completion.key)
            || self
                .functions
                .get(&completion.key)
                .map(|function| function.tier(completion.requested_tier).state)
                != Some(CompileState::Compiling(completion.requested_tier))
            || expected.attempt_id != completion.attempt_id
            || expected.tier != completion.requested_tier
            || expected.artifact_key != completion.artifact_key
        {
            self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
            return;
        }
        match completion.result {
            Ok(artifact) if completion.artifact_key == artifact.key() => {
                self.in_flight.remove(&completion.key);
                if let Some(record) = self.functions.get_mut(&completion.key) {
                    record.tier_mut(completion.requested_tier).state =
                        CompileState::Ready(completion.requested_tier);
                }
                if !invalidate::is_current_generation(&self.current_generations, completion.key) {
                    self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
                    self.retire_state(completion.key);
                    return;
                }
                let artifact_key = artifact.key();
                #[cfg(feature = "compiler")]
                let optimization_metrics = artifact.optimized_metadata().map(|metadata| {
                    (
                        metadata.boxes_elided(),
                        metadata.cse_eliminated(),
                        metadata.dead_nodes_eliminated(),
                    )
                });
                let dependency_versions = artifact
                    .dependencies()
                    .iter()
                    .map(|dependency| {
                        (
                            DependencyKey::function(dependency.function),
                            dependency.function.generation,
                        )
                    })
                    .collect::<Vec<_>>();
                if dependency_versions.iter().any(|(dependency, generation)| {
                    let DependencyKey::Function(function) = *dependency;
                    function.generation != *generation
                        || self.current_generations.get(&function.id) != Some(generation)
                }) {
                    self.record_failure(completion.key, completion.requested_tier);
                    return;
                }
                let mut staged_dependencies = self.dependencies.clone();
                if staged_dependencies
                    .install(
                        DependencyKey::function(completion.key),
                        completion.key.generation,
                        dependency_versions
                            .iter()
                            .map(|(dependency, _)| *dependency),
                    )
                    .is_err()
                {
                    self.record_failure(completion.key, completion.requested_tier);
                    return;
                }
                match install::publish(&mut self.cache, artifact) {
                    Ok(insert) => {
                        self.dependencies = staged_dependencies;
                        for evicted in insert.evictions() {
                            self.record_eviction(*evicted);
                        }
                        self.installed_keys
                            .insert((completion.key, completion.requested_tier), artifact_key);
                        if let Some(record) = self.functions.get_mut(&completion.key) {
                            let tier_record = record.tier_mut(completion.requested_tier);
                            tier_record.state = CompileState::Cold;
                            tier_record.attempts = 0;
                            record.published = Some(completion.requested_tier);
                        }
                        self.metrics.installed = self.metrics.installed.saturating_add(1);
                        #[cfg(feature = "compiler")]
                        if let Some((boxes, cse, dead)) = optimization_metrics {
                            self.metrics.boxes_elided =
                                self.metrics.boxes_elided.saturating_add(boxes);
                            self.metrics.cse_eliminated =
                                self.metrics.cse_eliminated.saturating_add(cse);
                            self.metrics.dead_nodes_eliminated =
                                self.metrics.dead_nodes_eliminated.saturating_add(dead);
                        }
                    }
                    Err(_) => self.record_failure(completion.key, completion.requested_tier),
                }
            }
            Ok(_) => {
                self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
            }
            Err(failure) => {
                self.in_flight.remove(&completion.key);
                if failure == CompileFailure::TimedOut {
                    self.metrics.compile_timeouts = self.metrics.compile_timeouts.saturating_add(1);
                }
                self.record_failure(completion.key, completion.requested_tier);
            }
        }
    }

    fn record_failure(&mut self, key: FunctionKey, tier: Tier) {
        self.metrics.compile_failures = self.metrics.compile_failures.saturating_add(1);
        let Some(record) = self.functions.get_mut(&key) else {
            return;
        };
        let tier_record = record.tier_mut(tier);
        tier_record.attempts = tier_record.attempts.saturating_add(1);
        if tier_record.attempts >= self.max_attempts {
            tier_record.state = CompileState::Blacklisted;
            self.metrics.blacklisted = self.metrics.blacklisted.saturating_add(1);
        } else {
            let retry_after = self.clock.saturating_add(u64::from(tier_record.attempts));
            tier_record.state = CompileState::Backoff {
                attempts: tier_record.attempts,
                retry_after,
            };
        }
    }

    fn retire_older_generations(&mut self, current: FunctionKey) {
        let older = self
            .functions
            .keys()
            .copied()
            .filter(|key| key.id == current.id && key.generation < current.generation)
            .collect::<Vec<_>>();
        for key in older {
            self.retire_state(key);
        }
    }

    fn retire_state(&mut self, key: FunctionKey) {
        self.queue.retain(|request| request.key != key);
        self.in_flight.remove(&key);
        let function = self.functions.entry(key).or_default();
        let was_retired = function.retired;
        function.retired = true;
        function.published = None;
        if !was_retired {
            self.metrics.retired = self.metrics.retired.saturating_add(1);
        }
        self.installed_keys
            .retain(|(installed_key, _), _| *installed_key != key);
        self.cache.invalidate_deferred(key);
    }

    fn record_eviction(&mut self, evicted: ArtifactKey) {
        let key = FunctionKey::new(evicted.function_id, evicted.generation);
        self.installed_keys.remove(&(key, evicted.tier));
        if let Some(record) = self.functions.get_mut(&key) {
            if record.published == Some(evicted.tier) {
                record.published = (evicted.tier == Tier::Optimizing
                    && self.installed_keys.contains_key(&(key, Tier::Baseline)))
                .then_some(Tier::Baseline);
            }
        }
        self.metrics.evicted = self.metrics.evicted.saturating_add(1);
    }

    pub fn retire(&mut self, key: FunctionKey) {
        let advances_watermark = self
            .current_generations
            .get(&key.id)
            .is_none_or(|generation| key.generation > *generation);
        if advances_watermark {
            self.retire_older_generations(key);
            self.current_generations.insert(key.id, key.generation);
        }
        let mut invalidated = self.dependencies.invalidate(DependencyKey::function(key));
        // A queued or compiling function has no dependency node yet. Retirement
        // must still retire the identity itself so a late worker completion can
        // never publish it.
        let own_dependency = DependencyKey::function(key);
        if !invalidated.contains(&own_dependency) {
            invalidated.push(own_dependency);
        }
        self.metrics.dependency_invalidations = self
            .metrics
            .dependency_invalidations
            .saturating_add(invalidated.len() as u64);
        for dependency in invalidated {
            let DependencyKey::Function(function) = dependency;
            self.retire_state(function);
        }
    }

    pub fn state(&self, key: FunctionKey) -> CompileState {
        self.functions
            .get(&key)
            .map_or(CompileState::Cold, FunctionState::visible_state)
    }

    pub fn tier_state(&self, key: FunctionKey, tier: Tier) -> CompileState {
        let Some(function) = self.functions.get(&key) else {
            return CompileState::Cold;
        };
        if function.retired {
            return CompileState::Retired;
        }
        let state = function.tier(tier).state;
        if state != CompileState::Cold {
            return state;
        }
        if self.installed_keys.contains_key(&(key, tier)) {
            CompileState::Installed(tier)
        } else {
            CompileState::Cold
        }
    }

    pub fn advance_clock(&mut self, now: u64) {
        self.clock = self.clock.max(now);
    }

    pub fn metrics(&self) -> JitMetrics {
        let mut metrics = self.metrics.clone();
        metrics.completion_queue_saturated =
            self.completion_signals.saturated.load(Ordering::Acquire);
        metrics.code_bytes = self.cache.charged_code_bytes();
        metrics.metadata_bytes = self.cache.charged_metadata_bytes();
        metrics
    }

    pub fn set_native_enabled(&mut self, enabled: bool) {
        self.metrics.set_native_enabled(enabled);
    }

    pub fn set_worker_usage(&mut self, jobs: usize, snapshots: usize, ir: usize) {
        self.metrics.pending_worker_jobs = jobs;
        self.metrics.pending_snapshot_bytes = snapshots;
        self.metrics.active_ir_bytes = ir;
    }

    pub fn record_resource_limit_rejection(&mut self) {
        self.metrics.resource_limit_rejections =
            self.metrics.resource_limit_rejections.saturating_add(1);
    }

    pub fn record_tier2_entry(&mut self) {
        self.metrics.tier2_entries = self.metrics.tier2_entries.saturating_add(1);
    }
    pub fn record_deopt(&mut self, guard_failure: bool) {
        self.metrics.deopts = self.metrics.deopts.saturating_add(1);
        self.metrics.side_exits = self.metrics.side_exits.saturating_add(1);
        if guard_failure {
            self.metrics.tier2_guard_failures = self.metrics.tier2_guard_failures.saturating_add(1);
        }
    }

    pub fn pin(&mut self, key: FunctionKey, tier: Tier) -> Option<ExecutionPin> {
        let artifact_key = *self.installed_keys.get(&(key, tier))?;
        self.cache.pin(artifact_key)
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn cache_bytes(&self) -> usize {
        self.cache.charged_bytes()
    }

    pub fn poll_cache_reclamation(&mut self) -> usize {
        self.cache.poll_reclamation()
    }

    pub fn completion_sender(&self) -> CompletionSender {
        CompletionSender {
            sender: self.completion_sender.clone(),
            signals: Arc::clone(&self.completion_signals),
        }
    }

    /// Applies worker completions on the caller's runtime-locked coordinator.
    pub fn drain_completions(&mut self) -> CompletionDrain {
        self.drain_completions_with_budget(DEFAULT_COMPLETION_DRAIN_BUDGET)
    }

    /// Applies at most `budget` worker completions without allocating an intermediate queue.
    pub fn drain_completions_with_budget(&mut self, budget: usize) -> CompletionDrain {
        let reclamation = self.cache.poll_reclamation_with_budget(budget);
        let reclaimed = reclamation.reclaimed();
        let completion_budget = budget.saturating_sub(reclaimed);
        let mut drained = 0;
        while drained < completion_budget {
            let completion = self
                .completion_receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            let Some(completion) = completion else {
                break;
            };
            self.completion_signals.decrement_pending();
            self.complete(completion);
            drained += 1;
        }
        CompletionDrain {
            drained,
            reclaimed,
            may_have_remaining: reclamation.may_have_remaining()
                || self.cache.reclamation_requested()
                || self.completion_signals.pending.load(Ordering::Acquire) != 0,
        }
    }

    pub fn shutdown(&mut self) {
        if self.shutdown {
            return;
        }
        self.shutdown = true;
        self.queue.clear();
        self.in_flight.clear();
        self.completion_receiver.take();
        self.completion_sender.take();
        self.completion_signals.pending.store(0, Ordering::Release);
        let functions = self.functions.keys().copied().collect::<Vec<_>>();
        for key in functions {
            self.retire_state(key);
        }
        self.cache.collect_invalidated();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        bytecode::{opcode, CompileSnapshot, VerifyLimits},
        compiler::{mock::FakeCompiler, Compiler},
    };

    fn snapshot() -> VerifiedFunction {
        CompileSnapshot::from_untrusted_bytecode(vec![opcode::RETURN_UNDEF], 0, 0, 0, 0)
            .verify(VerifyLimits::default())
            .unwrap()
    }

    #[test]
    fn mock_compiler_waits_for_release_before_submitting_completion() {
        let mut coordinator = Coordinator::with_limits(1, 1, 4, 3);
        let key = FunctionKey::new(1, 1);
        coordinator.queue(key, Tier::Baseline, snapshot()).unwrap();
        let request = coordinator.begin_next().unwrap();
        let sender = coordinator.completion_sender();
        let (compiler, control) = FakeCompiler::new(1);
        let compiler = Arc::new(compiler);
        let worker_compiler = Arc::clone(&compiler);
        let worker = std::thread::spawn(move || {
            compile_and_send(worker_compiler.as_ref(), request, &sender).unwrap()
        });

        let observed = control.next_request().expect("worker reached compiler");
        assert_eq!(observed.key(), key);
        assert_eq!(coordinator.drain_completions().drained(), 0);
        control.complete(CompiledArtifact::fake(Tier::Baseline));
        worker.join().unwrap();

        assert_eq!(coordinator.drain_completions().drained(), 1);
        assert_eq!(
            coordinator.state(key),
            CompileState::Installed(Tier::Baseline)
        );
    }

    struct PanickingCompiler;

    impl Compiler for PanickingCompiler {
        fn compile(&self, _request: CompileRequest) -> Result<CompiledArtifact, CompileFailure> {
            panic!("deterministic compiler panic")
        }
    }

    #[test]
    fn compiler_panic_becomes_failure_completion_without_unwinding() {
        let mut coordinator = Coordinator::with_limits(1, 1, 4, 3);
        let key = FunctionKey::new(1, 1);
        coordinator.queue(key, Tier::Baseline, snapshot()).unwrap();
        let request = coordinator.begin_next().unwrap();

        compile_and_send(
            &PanickingCompiler,
            request,
            &coordinator.completion_sender(),
        )
        .unwrap();
        assert_eq!(coordinator.drain_completions().drained(), 1);
        assert!(matches!(
            coordinator.state(key),
            CompileState::Backoff { attempts: 1, .. }
        ));
        assert_eq!(coordinator.metrics().compile_failures, 1);
    }
}
