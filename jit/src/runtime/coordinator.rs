use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
};

use crate::{
    bytecode::VerifiedFunction,
    code_cache::{ArtifactKey, ArtifactVersionIdentity, CodeCache, CompiledArtifact, ExecutionPin},
    compiler::CompileFailure,
    JitMetrics,
};

use super::{
    install, invalidate, BoundedSpecializationSignature, CallSpecializationKey, DependencyGraph,
    DependencyKey, FeedbackRepresentation, FeedbackSnapshot, ObservedType,
};

fn call_specialization_fingerprint(key: &CallSpecializationKey) -> u64 {
    fn mix(state: u64, value: u64) -> u64 {
        (state ^ value).wrapping_mul(0x100_0000_01b3)
    }
    fn representation_tag(representation: FeedbackRepresentation) -> u64 {
        match representation {
            FeedbackRepresentation::Int32 => 1,
            FeedbackRepresentation::Float64 => 2,
            FeedbackRepresentation::HeapRef => 3,
        }
    }
    let mut state = 0xcbf2_9ce4_8422_2325;
    state = mix(state, key.caller().id);
    state = mix(state, key.caller().generation);
    state = mix(state, key.callee().id);
    state = mix(state, key.callee().generation);
    state = mix(state, key.callee_identity());
    state = mix(state, key.callee_bytecode_identity());
    state = mix(state, key.arity() as u64);
    for argument in key.arguments() {
        state = mix(state, representation_tag(*argument));
    }
    state = mix(state, representation_tag(key.result()));
    mix(state, key.feedback_epoch())
}

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

/// A version-pinned callee selected for a monomorphic compiled call edge.
///
/// Holding this value keeps the callee's code alive across concurrent
/// invalidation. New resolutions still fail as soon as the generation is
/// retired, so stale callers cannot acquire another target.
#[derive(Debug)]
pub struct CompiledCallTarget {
    identity: ArtifactVersionIdentity,
    call: CallSpecializationKey,
    pin: ExecutionPin,
}

/// Executable scalar entry retained by a caller compilation request.  The
/// cloned publication owns the executable allocation, closing the race where
/// a callee is retired while its caller is still compiling.
#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
#[derive(Clone, Debug)]
pub struct DirectCallTarget {
    pc: u32,
    call: CallSpecializationKey,
    signature: BoundedSpecializationSignature,
    published: crate::compiler::baseline::PublishedBaselineCode,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
impl DirectCallTarget {
    pub const fn pc(&self) -> u32 {
        self.pc
    }
    pub const fn call(&self) -> &CallSpecializationKey {
        &self.call
    }
    pub const fn signature(&self) -> &BoundedSpecializationSignature {
        &self.signature
    }
    pub fn entry(&self) -> *const u8 {
        self.published.as_ptr()
    }
    pub(crate) fn publication(&self) -> crate::compiler::baseline::PublishedBaselineCode {
        self.published.clone()
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn artifact_matches_direct_call(artifact: &CompiledArtifact, call: &CallSpecializationKey) -> bool {
    artifact.direct_call_published().is_some()
        && artifact
            .optimized_metadata()
            .and_then(|metadata| metadata.direct_call_signature())
            .is_some_and(|signature| {
                signature.function() == call.callee()
                    && signature.arguments() == call.arguments()
                    && signature.result() == call.result()
            })
}

impl CompiledCallTarget {
    pub const fn identity(&self) -> ArtifactVersionIdentity {
        self.identity
    }

    pub const fn call(&self) -> &CallSpecializationKey {
        &self.call
    }

    pub fn artifact(&self) -> &CompiledArtifact {
        self.pin.artifact()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompiledCallTargetError {
    InvalidSpecialization(QueueError),
    StaleCallee,
    CalleeNotInstalled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GuardId(u32);

impl GuardId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SidePathProfile {
    function: FunctionKey,
    guard: GuardId,
    pc: u32,
    observed: ObservedType,
    feedback_epoch: u64,
}

impl SidePathProfile {
    pub const fn new(
        function: FunctionKey,
        guard: GuardId,
        pc: u32,
        observed: ObservedType,
        feedback_epoch: u64,
    ) -> Self {
        Self {
            function,
            guard,
            pc,
            observed,
            feedback_epoch,
        }
    }
    pub const fn function(self) -> FunctionKey {
        self.function
    }
    pub const fn guard(self) -> GuardId {
        self.guard
    }
    pub const fn pc(self) -> u32 {
        self.pc
    }
    pub const fn observed(self) -> ObservedType {
        self.observed
    }
    pub const fn feedback_epoch(self) -> u64 {
        self.feedback_epoch
    }
    fn fingerprint(self) -> u64 {
        let observed = match self.observed {
            ObservedType::Int32 => 1,
            ObservedType::Float64 => 2,
            ObservedType::Bool => 3,
            ObservedType::Null => 4,
            ObservedType::Undefined => 5,
            ObservedType::String => 6,
            ObservedType::Object => 7,
            ObservedType::Function(key) => 8 ^ key.id ^ key.generation.rotate_left(17),
            ObservedType::BigInt => 9,
            ObservedType::Symbol => 10,
        };
        self.function.id
            ^ self.function.generation.rotate_left(7)
            ^ u64::from(self.guard.0).rotate_left(13)
            ^ u64::from(self.pc).rotate_left(29)
            ^ observed
            ^ self.feedback_epoch.rotate_left(41)
    }

    fn signature_fingerprint(self) -> u64 {
        let mut signature = self;
        signature.feedback_epoch = 0;
        signature.fingerprint()
    }
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
    feedback: Arc<FeedbackSnapshot>,
    side_path_profile: Option<SidePathProfile>,
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    direct_call_targets: Arc<[DirectCallTarget]>,
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

    pub fn feedback(&self) -> &FeedbackSnapshot {
        &self.feedback
    }
    pub const fn side_path_profile(&self) -> Option<SidePathProfile> {
        self.side_path_profile
    }
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub fn direct_call_target(&self, pc: u32) -> Option<&DirectCallTarget> {
        self.direct_call_targets.iter().find(|target| {
            self.feedback.call_specialization_at(self.key, pc).as_ref() == Some(target.call())
        })
    }
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub(crate) fn direct_call_targets(&self) -> &[DirectCallTarget] {
        &self.direct_call_targets
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
    // `Atomic::try_update` would avoid this deprecation, but it is unavailable
    // on the crate's Rust 1.87 MSRV.
    #[allow(deprecated)]
    fn increment_pending(&self) {
        let _ = self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                Some(pending.saturating_add(1))
            });
    }

    // See `increment_pending` for the MSRV compatibility rationale.
    #[allow(deprecated)]
    fn decrement_pending(&self) {
        let _ = self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                Some(pending.saturating_sub(1))
            });
    }

    // See `increment_pending` for the MSRV compatibility rationale.
    #[allow(deprecated)]
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
    feedback_epoch: u64,
    side_path: bool,
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
    instability_attempts: u8,
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
    latest_feedback_epochs: HashMap<FunctionKey, u64>,
    side_exits: HashMap<FunctionKey, HashMap<u32, u8>>,
    side_exit_observations: HashMap<(FunctionKey, u32), Option<ObservedType>>,
    specialization_versions: HashMap<(FunctionKey, u64), u8>,
    call_specialization_versions: HashMap<FunctionKey, HashMap<u64, u8>>,
    profitability_demotions: HashSet<FunctionKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideExitAction {
    Counted,
    StablePathThreshold,
    Demote { retry_after: u64 },
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
            latest_feedback_epochs: HashMap::new(),
            side_exits: HashMap::new(),
            side_exit_observations: HashMap::new(),
            specialization_versions: HashMap::new(),
            call_specialization_versions: HashMap::new(),
            profitability_demotions: HashSet::new(),
        }
    }

    /// Registers bounded version identity without claiming a direct-call path.
    pub fn register_call_specialization(
        &mut self,
        primary: &BoundedSpecializationSignature,
        call: &CallSpecializationKey,
    ) -> Result<ArtifactVersionIdentity, QueueError> {
        if primary.function() != call.caller()
            || primary.feedback_epoch() == 0
            || primary.feedback_epoch() != call.feedback_epoch()
        {
            return Err(QueueError::SnapshotIdentity);
        }
        let identity = ArtifactVersionIdentity::new(
            primary.fingerprint(),
            call_specialization_fingerprint(call),
        );
        let versions = self
            .call_specialization_versions
            .entry(call.caller())
            .or_default();
        if let Some(attempts) = versions.get_mut(&identity.fingerprint()) {
            if *attempts >= self.max_attempts {
                return Err(QueueError::Blacklisted);
            }
            *attempts = attempts.saturating_add(1);
            return Ok(identity);
        }
        if versions.len() >= usize::from(self.max_attempts) {
            return Err(QueueError::Blacklisted);
        }
        versions.insert(identity.fingerprint(), 1);
        Ok(identity)
    }

    /// Resolves a stable call specialization to a pinned optimizing artifact.
    ///
    /// This is the safety boundary needed by compiled-to-compiled lowering:
    /// identity validation and bounded versioning happen before the callee is
    /// looked up, and the returned execution pin prevents reclamation while a
    /// caller is using the target. The current compiler still uses the generic
    /// CALL helper until its native frame ABI can consume this target.
    pub fn resolve_compiled_call_target(
        &mut self,
        primary: &BoundedSpecializationSignature,
        call: &CallSpecializationKey,
    ) -> Result<CompiledCallTarget, CompiledCallTargetError> {
        if primary.function() != call.caller()
            || primary.feedback_epoch() == 0
            || primary.feedback_epoch() != call.feedback_epoch()
        {
            return Err(CompiledCallTargetError::InvalidSpecialization(
                QueueError::SnapshotIdentity,
            ));
        }
        if self.current_generations.get(&call.callee().id) != Some(&call.callee().generation)
            || self
                .functions
                .get(&call.callee())
                .is_some_and(|function| function.retired)
        {
            return Err(CompiledCallTargetError::StaleCallee);
        }
        let pin = self
            .pin(call.callee(), Tier::Optimizing)
            .ok_or(CompiledCallTargetError::CalleeNotInstalled)?;
        // Missing or stale callees are availability failures, not unstable
        // specialization attempts, so consume the bounded version budget only
        // once an executable target has actually been acquired.
        let identity = self
            .register_call_specialization(primary, call)
            .map_err(CompiledCallTargetError::InvalidSpecialization)?;
        Ok(CompiledCallTarget {
            identity,
            call: call.clone(),
            pin,
        })
    }

    pub fn queue(
        &mut self,
        key: FunctionKey,
        tier: Tier,
        snapshot: VerifiedFunction,
    ) -> Result<(), QueueError> {
        self.queue_with_feedback(key, tier, snapshot, FeedbackSnapshot::empty(self.clock))
    }

    pub fn queue_with_feedback(
        &mut self,
        key: FunctionKey,
        tier: Tier,
        snapshot: VerifiedFunction,
        feedback: FeedbackSnapshot,
    ) -> Result<(), QueueError> {
        self.queue_request(key, tier, snapshot, feedback, None)
    }

    pub fn queue_side_path(
        &mut self,
        key: FunctionKey,
        snapshot: VerifiedFunction,
        feedback: FeedbackSnapshot,
        profile: SidePathProfile,
    ) -> Result<(), QueueError> {
        if profile.function != key
            || profile.feedback_epoch == 0
            || profile.feedback_epoch != feedback.epoch()
            || !feedback.contains_stable_observation(key, profile.pc, profile.observed)
            || !self.installed_keys.contains_key(&(key, Tier::Baseline))
            || !self.installed_keys.contains_key(&(key, Tier::Optimizing))
        {
            return Err(QueueError::SnapshotIdentity);
        }
        self.queue_request(key, Tier::Optimizing, snapshot, feedback, Some(profile))
    }

    fn queue_request(
        &mut self,
        key: FunctionKey,
        tier: Tier,
        snapshot: VerifiedFunction,
        feedback: FeedbackSnapshot,
        side_path_profile: Option<SidePathProfile>,
    ) -> Result<(), QueueError> {
        if self.shutdown {
            return Err(QueueError::Shutdown);
        }
        let side_path_signature = side_path_profile.map(SidePathProfile::signature_fingerprint);
        if side_path_signature.is_some_and(|signature| {
            self.specialization_versions
                .get(&(key, signature))
                .copied()
                .unwrap_or(0)
                >= self.max_attempts
        }) {
            return Err(QueueError::Blacklisted);
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
        let interpreter_deopt_target = self
            .functions
            .get(&key)
            .is_some_and(|function| function.baseline.state == CompileState::Blacklisted);
        let installed_optimizing = self.installed_keys.contains_key(&(key, Tier::Optimizing));
        match tier {
            Tier::Baseline if installed_baseline || installed_optimizing => {
                return Err(QueueError::NotReady)
            }
            Tier::Optimizing
                if (!installed_baseline && !interpreter_deopt_target)
                    || (installed_optimizing && side_path_profile.is_none()) =>
            {
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
            CompileState::Installed(Tier::Optimizing) if side_path_profile.is_some() => {}
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
        let primary_feedback_signature = (tier == Tier::Optimizing && side_path_profile.is_none())
            .then(|| feedback.bounded_specialization(key))
            .flatten()
            .map(|signature| signature.fingerprint());
        let call_feedback_signature = (tier == Tier::Optimizing && side_path_profile.is_none())
            .then(|| {
                snapshot
                    .instructions()
                    .iter()
                    .filter_map(|instruction| {
                        feedback.call_specialization_at(key, instruction.pc())
                    })
                    .map(|call| call_specialization_fingerprint(&call))
                    .reduce(|prior, fingerprint| prior.rotate_left(17) ^ fingerprint)
            })
            .flatten();
        let feedback_signature = match (primary_feedback_signature, call_feedback_signature) {
            (Some(primary), Some(call)) => {
                Some(ArtifactVersionIdentity::new(primary, call).fingerprint())
            }
            (Some(primary), None) => Some(primary),
            (None, Some(call)) => Some(ArtifactVersionIdentity::new(0, call).fingerprint()),
            (None, None) => None,
        };
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
            specialization_fingerprint: side_path_profile
                .map(SidePathProfile::fingerprint)
                .or(feedback_signature)
                .unwrap_or(0),
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
        let feedback_epoch = feedback.epoch();
        if tier == Tier::Optimizing && side_path_profile.is_none() {
            self.latest_feedback_epochs
                .entry(key)
                .and_modify(|epoch| *epoch = (*epoch).max(feedback_epoch))
                .or_insert(feedback_epoch);
        }
        #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
        let direct_call_targets =
            if matches!(tier, Tier::Baseline | Tier::Optimizing) && side_path_profile.is_none() {
                snapshot
                    .instructions()
                    .iter()
                    .filter_map(|instruction| {
                        let call = feedback.call_specialization_at(key, instruction.pc())?;
                        let pin = self
                            .pin(call.callee(), Tier::Optimizing)
                            .or_else(|| self.pin(call.callee(), Tier::Baseline))?;
                        let artifact = pin.artifact();
                        let signature = artifact
                            .optimized_metadata()?
                            .direct_call_signature()?
                            .clone();
                        if signature.function() != call.callee()
                            || signature.arguments() != call.arguments()
                            || signature.result() != call.result()
                        {
                            return None;
                        }
                        let published = artifact.direct_call_published()?.clone();
                        Some(DirectCallTarget {
                            pc: instruction.pc(),
                            call,
                            signature,
                            published,
                        })
                    })
                    .collect::<Vec<_>>()
                    .into()
            } else {
                Arc::from([])
            };
        self.queue.push_back(CompileRequest {
            key,
            tier,
            snapshot,
            artifact_key,
            attempt_id: AttemptId(next_attempt_id),
            feedback_epoch,
            feedback: Arc::new(feedback),
            side_path_profile,
            #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
            direct_call_targets,
        });
        if let Some(signature) = side_path_signature {
            let versions = self
                .specialization_versions
                .entry((key, signature))
                .or_default();
            *versions = versions.saturating_add(1);
        }
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
                    feedback_epoch: request.feedback_epoch,
                    side_path: request.side_path_profile.is_some(),
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
                #[cfg(feature = "compiler")]
                if completion.requested_tier == Tier::Optimizing
                    && artifact.optimized_metadata().is_some_and(|metadata| {
                        metadata.feedback_epoch() != expected.feedback_epoch
                            || (!expected.side_path
                                && self.latest_feedback_epochs.get(&completion.key)
                                    != Some(&expected.feedback_epoch))
                    })
                {
                    self.in_flight.remove(&completion.key);
                    self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
                    self.record_invalid_artifact(completion.key, completion.requested_tier);
                    return;
                }
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
                if dependency_versions
                    .iter()
                    .any(|(dependency, generation)| match *dependency {
                        DependencyKey::Function(function) => {
                            function.generation != *generation
                                || self.current_generations.get(&function.id) != Some(generation)
                        }
                        DependencyKey::Shape(_) | DependencyKey::Prototype(_) => false,
                    })
                {
                    self.record_invalid_artifact(completion.key, completion.requested_tier);
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
                    self.record_install_failure(completion.key, completion.requested_tier);
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
                        if expected.side_path {
                            self.latest_feedback_epochs
                                .insert(completion.key, expected.feedback_epoch);
                        }
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
                    Err(_) => {
                        self.record_install_failure(completion.key, completion.requested_tier)
                    }
                }
            }
            Ok(_) => {
                self.metrics.invalid_artifacts = self.metrics.invalid_artifacts.saturating_add(1);
                self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
            }
            Err(failure) => {
                self.in_flight.remove(&completion.key);
                self.record_compile_failure(completion.key, completion.requested_tier, failure);
            }
        }
    }

    fn record_compile_failure(&mut self, key: FunctionKey, tier: Tier, failure: CompileFailure) {
        let category = match failure {
            CompileFailure::UnsupportedOpcode => &mut self.metrics.unsupported_opcode_failures,
            CompileFailure::Tier1Rejected(_) => &mut self.metrics.tier1_rejections,
            CompileFailure::ResourceLimit => &mut self.metrics.resource_limit_failures,
            CompileFailure::TimedOut => &mut self.metrics.compile_timeouts,
            CompileFailure::Cancelled => &mut self.metrics.cancelled_compilations,
            CompileFailure::CompilerPanicked => &mut self.metrics.compiler_panics,
            CompileFailure::InvalidArtifact => &mut self.metrics.invalid_artifacts,
        };
        *category = category.saturating_add(1);
        self.record_failure(key, tier);
    }

    pub(crate) fn reject_tier1(
        &mut self,
        key: FunctionKey,
        tier: Tier,
        reason: crate::bytecode::FallbackReason,
    ) {
        if self
            .current_generations
            .get(&key.id)
            .is_none_or(|generation| key.generation > *generation)
        {
            self.retire_older_generations(key);
            self.current_generations.insert(key.id, key.generation);
        }
        self.functions.entry(key).or_default();
        self.record_compile_failure(key, tier, CompileFailure::Tier1Rejected(reason));
        let tier_record = self
            .functions
            .get_mut(&key)
            .expect("Tier 1 rejection registered the function")
            .tier_mut(tier);
        if tier_record.state != CompileState::Blacklisted {
            tier_record.state = CompileState::Blacklisted;
            self.metrics.blacklisted = self.metrics.blacklisted.saturating_add(1);
        }
    }

    fn record_invalid_artifact(&mut self, key: FunctionKey, tier: Tier) {
        self.metrics.invalid_artifacts = self.metrics.invalid_artifacts.saturating_add(1);
        self.record_failure(key, tier);
    }

    fn record_install_failure(&mut self, key: FunctionKey, tier: Tier) {
        self.metrics.install_failures = self.metrics.install_failures.saturating_add(1);
        self.record_failure(key, tier);
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
            if let DependencyKey::Function(function) = dependency {
                self.retire_state(function);
            }
        }
    }

    /// Unpublishes a harmful baseline version so future entries stay in the
    /// interpreter. Automatic tiering uses this only after its bounded
    /// profitability retries are exhausted; BaselineOnly never calls it.
    pub fn demote_baseline_to_interpreter(&mut self, key: FunctionKey) -> bool {
        let Some(function) = self.functions.get_mut(&key) else {
            return false;
        };
        if self.installed_keys.remove(&(key, Tier::Baseline)).is_none() {
            return false;
        }
        function.baseline.state = CompileState::Blacklisted;
        self.profitability_demotions.insert(key);
        if function.published == Some(Tier::Baseline) {
            function.published = None;
        }
        /* Keep the unpublished Baseline artifact resident as the cache's
         * lifetime pin while a bounded Tier2 trial is compiled. It is no
         * longer addressable through `installed_keys` or `published`, so it
         * cannot execute; optimized side exits resume the interpreter. */
        self.metrics.interpreter_demotions = self.metrics.interpreter_demotions.saturating_add(1);
        self.metrics.blacklisted = self.metrics.blacklisted.saturating_add(1);
        true
    }

    /// Returns true when an installed baseline caller has a stable call edge
    /// whose scalar callee entry is now published, but the caller artifact was
    /// compiled before that dependency became available.
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub fn direct_call_ready(&mut self, call: &CallSpecializationKey) -> bool {
        self.pin(call.callee(), Tier::Optimizing)
            .or_else(|| self.pin(call.callee(), Tier::Baseline))
            .is_some_and(|pin| artifact_matches_direct_call(pin.artifact(), call))
    }

    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub fn baseline_direct_refresh_ready(
        &mut self,
        key: FunctionKey,
        feedback: &FeedbackSnapshot,
    ) -> bool {
        let Some(caller_pin) = self.pin(key, Tier::Baseline) else {
            return false;
        };
        let installed_dependencies = caller_pin
            .artifact()
            .dependencies()
            .iter()
            .map(|dependency| dependency.function)
            .collect::<std::collections::HashSet<_>>();
        drop(caller_pin);
        feedback.call_specializations_for(key).any(|call| {
            if installed_dependencies.contains(&call.callee()) {
                return false;
            }
            let callee_pin = self
                .pin(call.callee(), Tier::Optimizing)
                .or_else(|| self.pin(call.callee(), Tier::Baseline));
            callee_pin.is_some_and(|pin| artifact_matches_direct_call(pin.artifact(), &call))
        })
    }

    /// Atomically makes an installed baseline caller queueable again. Existing
    /// executions retain their publication pin; new entries use the
    /// interpreter until the refreshed artifact is installed.
    pub fn prepare_baseline_direct_refresh(&mut self, key: FunctionKey) -> bool {
        let Some(function) = self.functions.get_mut(&key) else {
            return false;
        };
        if function.published != Some(Tier::Baseline)
            || !self.installed_keys.contains_key(&(key, Tier::Baseline))
        {
            return false;
        }
        self.installed_keys.remove(&(key, Tier::Baseline));
        function.baseline.state = CompileState::Cold;
        function.published = None;
        true
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

    /// Reports immutable generations that cannot publish either native tier.
    /// Profitability demotions remain probeable for their bounded Tier2 trial.
    pub fn is_terminally_blacklisted(&self, key: FunctionKey) -> bool {
        matches!(
            self.tier_state(key, Tier::Optimizing),
            CompileState::Blacklisted
        ) || (matches!(
            self.tier_state(key, Tier::Baseline),
            CompileState::Blacklisted
        ) && !self.profitability_demotions.contains(&key))
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

    /// Feeds observed time saved by an installed artifact into cache eviction.
    pub fn record_benefit(&mut self, key: FunctionKey, tier: Tier, saved_ns: u64) -> bool {
        let Some(artifact) = self.installed_keys.get(&(key, tier)).copied() else {
            return false;
        };
        self.cache.record_benefit(artifact, saved_ns).is_ok()
    }
    pub fn record_side_path_entries(&mut self, count: u64) {
        self.metrics.side_path_entries = self.metrics.side_path_entries.saturating_add(count);
    }
    pub fn record_deopt(&mut self, guard_failure: bool) {
        self.metrics.deopts = self.metrics.deopts.saturating_add(1);
        self.metrics.side_exits = self.metrics.side_exits.saturating_add(1);
        if guard_failure {
            self.metrics.tier2_guard_failures = self.metrics.tier2_guard_failures.saturating_add(1);
        }
    }

    pub fn record_optimized_side_exit(&mut self, key: FunctionKey, guard: u32) -> SideExitAction {
        self.record_optimized_side_exit_profile(key, guard, None)
    }

    pub fn record_optimized_side_exit_profile(
        &mut self,
        key: FunctionKey,
        guard: u32,
        observed: Option<ObservedType>,
    ) -> SideExitAction {
        self.record_deopt(true);
        let observation_key = (key, guard);
        let observation_changed = self
            .side_exit_observations
            .get(&observation_key)
            .is_some_and(|prior| *prior != observed);
        self.side_exit_observations
            .entry(observation_key)
            .or_insert(observed);
        let exits = self.side_exits.entry(key).or_default();
        let count = exits.entry(guard).or_default();
        *count = count.saturating_add(1);
        let count = *count;
        if exits.len() > 1 || observation_changed {
            let function = self.functions.entry(key).or_default();
            function.instability_attempts = function.instability_attempts.saturating_add(1);
            let attempts = function.instability_attempts;
            let delay = 1u64
                .checked_shl(u32::from(attempts.min(20)))
                .unwrap_or(u64::MAX);
            let retry_after = self.clock.saturating_add(delay);
            function.optimizing.state = if attempts >= self.max_attempts {
                self.metrics.blacklisted = self.metrics.blacklisted.saturating_add(1);
                CompileState::Blacklisted
            } else {
                CompileState::Backoff {
                    attempts,
                    retry_after,
                }
            };
            self.installed_keys.remove(&(key, Tier::Optimizing));
            if let Some(function) = self.functions.get_mut(&key) {
                function.published = Some(Tier::Baseline);
            }
            self.metrics.optimized_demotions = self.metrics.optimized_demotions.saturating_add(1);
            SideExitAction::Demote { retry_after }
        } else if count == 10 {
            SideExitAction::StablePathThreshold
        } else {
            SideExitAction::Counted
        }
    }

    /// Atomically returns an installed optimizing tier to a queueable state
    /// while preserving its baseline deopt target.
    pub fn prepare_stable_path_recompile(&mut self, key: FunctionKey) -> bool {
        if !self.installed_keys.contains_key(&(key, Tier::Baseline))
            || self
                .installed_keys
                .remove(&(key, Tier::Optimizing))
                .is_none()
        {
            return false;
        }
        if let Some(function) = self.functions.get_mut(&key) {
            function.optimizing.state = CompileState::Cold;
            function.published = Some(Tier::Baseline);
        }
        true
    }

    pub fn pin(&mut self, key: FunctionKey, tier: Tier) -> Option<ExecutionPin> {
        let artifact_key = *self.installed_keys.get(&(key, tier))?;
        let pin = self.cache.pin(artifact_key)?;
        #[cfg(feature = "compiler")]
        if tier == Tier::Optimizing
            && pin.artifact().optimized_metadata().is_some_and(|metadata| {
                self.latest_feedback_epochs.get(&key) != Some(&metadata.feedback_epoch())
            })
        {
            return None;
        }
        Some(pin)
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
