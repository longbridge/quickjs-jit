use std::{
    collections::{HashMap, VecDeque},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
};

use crate::{
    bytecode::VerifiedFunction,
    code_cache::{ArtifactKey, CodeCache, CompiledArtifact, ExecutionPin},
    compiler::CompileFailure,
    JitMetrics,
};

use super::{install, invalidate};

pub fn compile_and_send<C: crate::compiler::Compiler + ?Sized>(
    compiler: &C,
    request: CompileRequest,
    sender: &CompletionSender,
) -> Result<(), Box<CompileCompletion>> {
    let key = request.key();
    let requested_tier = request.tier();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compiler.compile(request)))
            .unwrap_or(Err(CompileFailure::CompilerPanicked));
    sender.send(CompileCompletion {
        key,
        requested_tier,
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

#[derive(Clone, Debug)]
pub struct CompileRequest {
    key: FunctionKey,
    tier: Tier,
    snapshot: VerifiedFunction,
    artifact_key: ArtifactKey,
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
}

#[derive(Debug)]
pub struct CompileCompletion {
    pub key: FunctionKey,
    pub requested_tier: Tier,
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
}

impl CompletionSender {
    pub fn try_send(&self, completion: CompileCompletion) -> Result<(), CompletionSendError> {
        let Some(sender) = &self.sender else {
            return Err(CompletionSendError::Closed(Box::new(completion)));
        };
        sender.try_send(completion).map_err(|error| match error {
            TrySendError::Full(completion) => CompletionSendError::Full(Box::new(completion)),
            TrySendError::Disconnected(completion) => {
                CompletionSendError::Closed(Box::new(completion))
            }
        })
    }

    pub fn send(&self, completion: CompileCompletion) -> Result<(), Box<CompileCompletion>> {
        let Some(sender) = &self.sender else {
            return Err(Box::new(completion));
        };
        sender.send(completion).map_err(|error| Box::new(error.0))
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
}

#[derive(Debug)]
struct FunctionState {
    state: CompileState,
    attempts: u8,
}

#[derive(Debug)]
pub struct Coordinator {
    max_queue_len: usize,
    max_attempts: u8,
    clock: u64,
    queue: VecDeque<CompileRequest>,
    functions: HashMap<FunctionKey, FunctionState>,
    current_generations: HashMap<u64, u64>,
    in_flight: HashMap<FunctionKey, ArtifactKey>,
    metrics: JitMetrics,
    completion_sender: Option<SyncSender<CompileCompletion>>,
    completion_receiver: Option<Receiver<CompileCompletion>>,
    shutdown: bool,
    cache: CodeCache,
    installed_keys: HashMap<(FunctionKey, Tier), ArtifactKey>,
    environment: ArtifactEnvironment,
}

impl Coordinator {
    pub fn with_limits(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        max_artifacts: usize,
    ) -> Self {
        Self::with_environment(
            max_queue_len,
            max_completion_len,
            max_attempts,
            max_artifacts,
            ArtifactEnvironment::default(),
        )
    }

    pub fn with_environment(
        max_queue_len: usize,
        max_completion_len: usize,
        max_attempts: u8,
        max_artifacts: usize,
        environment: ArtifactEnvironment,
    ) -> Self {
        let (completion_sender, completion_receiver) = mpsc::sync_channel(max_completion_len);
        Self {
            max_queue_len,
            max_attempts,
            clock: 0,
            queue: VecDeque::new(),
            functions: HashMap::new(),
            current_generations: HashMap::new(),
            in_flight: HashMap::new(),
            metrics: JitMetrics::disabled(),
            completion_sender: Some(completion_sender),
            completion_receiver: Some(completion_receiver),
            shutdown: false,
            cache: CodeCache::new(max_artifacts),
            installed_keys: HashMap::new(),
            environment,
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
        match self.state(key) {
            CompileState::Cold => {}
            CompileState::Backoff { retry_after, .. } if self.clock >= retry_after => {}
            CompileState::Installed(Tier::Baseline) if tier == Tier::Optimizing => {}
            CompileState::Retired => return Err(QueueError::Retired),
            CompileState::Blacklisted => return Err(QueueError::Blacklisted),
            _ => return Err(QueueError::NotReady),
        }
        if self.queue.len() >= self.max_queue_len {
            self.metrics.queue_saturated = self.metrics.queue_saturated.saturating_add(1);
            return Err(QueueError::Full);
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
        self.queue.push_back(CompileRequest {
            key,
            tier,
            snapshot,
            artifact_key,
        });
        let attempts = self.functions.get(&key).map_or(0, |record| record.attempts);
        self.functions.insert(
            key,
            FunctionState {
                state: CompileState::Queued(tier),
                attempts,
            },
        );
        self.metrics.queued = self.metrics.queued.saturating_add(1);
        Ok(())
    }

    pub fn begin_next(&mut self) -> Option<CompileRequest> {
        loop {
            let request = self.queue.pop_front()?;
            let Some(record) = self.functions.get_mut(&request.key) else {
                continue;
            };
            if record.state != CompileState::Queued(request.tier) {
                continue;
            }
            record.state = CompileState::Compiling(request.tier);
            self.in_flight.insert(request.key, request.artifact_key);
            self.metrics.compiling = self.metrics.compiling.saturating_add(1);
            return Some(request);
        }
    }

    pub fn complete(&mut self, completion: CompileCompletion) {
        if !invalidate::is_current_generation(&self.current_generations, completion.key)
            || self.state(completion.key) != CompileState::Compiling(completion.requested_tier)
        {
            self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
            return;
        }
        let expected = self.in_flight.remove(&completion.key);
        match completion.result {
            Ok(artifact) if expected == Some(artifact.key()) => {
                if let Some(record) = self.functions.get_mut(&completion.key) {
                    record.state = CompileState::Ready(completion.requested_tier);
                }
                if !invalidate::is_current_generation(&self.current_generations, completion.key) {
                    self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
                    self.retire_state(completion.key);
                    return;
                }
                let artifact_key = artifact.key();
                match install::publish(&mut self.cache, artifact) {
                    Ok(insert) => {
                        if let Some(evicted) = insert.evicted() {
                            self.record_eviction(evicted);
                        }
                        self.installed_keys
                            .insert((completion.key, completion.requested_tier), artifact_key);
                        if let Some(record) = self.functions.get_mut(&completion.key) {
                            record.state = CompileState::Installed(completion.requested_tier);
                        }
                        self.metrics.installed = self.metrics.installed.saturating_add(1);
                    }
                    Err(_) => self.record_failure(completion.key),
                }
            }
            Ok(_) => {
                self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
                self.record_failure(completion.key);
            }
            Err(_) => {
                self.record_failure(completion.key);
            }
        }
    }

    fn record_failure(&mut self, key: FunctionKey) {
        self.metrics.compile_failures = self.metrics.compile_failures.saturating_add(1);
        let Some(record) = self.functions.get_mut(&key) else {
            return;
        };
        record.attempts = record.attempts.saturating_add(1);
        if record.attempts >= self.max_attempts {
            record.state = CompileState::Blacklisted;
            self.metrics.blacklisted = self.metrics.blacklisted.saturating_add(1);
        } else {
            let retry_after = self.clock.saturating_add(u64::from(record.attempts));
            record.state = CompileState::Backoff {
                attempts: record.attempts,
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
        let attempts = self.functions.get(&key).map_or(0, |record| record.attempts);
        let was_retired = self.state(key) == CompileState::Retired;
        self.functions.insert(
            key,
            FunctionState {
                state: CompileState::Retired,
                attempts,
            },
        );
        if !was_retired {
            self.metrics.retired = self.metrics.retired.saturating_add(1);
        }
        self.installed_keys
            .retain(|(installed_key, _), _| *installed_key != key);
        self.cache.invalidate(key);
    }

    fn record_eviction(&mut self, evicted: ArtifactKey) {
        let key = FunctionKey::new(evicted.function_id, evicted.generation);
        self.installed_keys.remove(&(key, evicted.tier));
        if self.state(key) == CompileState::Installed(evicted.tier) {
            let fallback = if evicted.tier == Tier::Optimizing
                && self.installed_keys.contains_key(&(key, Tier::Baseline))
            {
                CompileState::Installed(Tier::Baseline)
            } else {
                CompileState::Cold
            };
            if let Some(record) = self.functions.get_mut(&key) {
                record.state = fallback;
            }
        }
        self.metrics.evicted = self.metrics.evicted.saturating_add(1);
    }

    pub fn retire(&mut self, key: FunctionKey) {
        self.current_generations
            .entry(key.id)
            .and_modify(|generation| *generation = (*generation).max(key.generation))
            .or_insert(key.generation);
        self.retire_state(key);
    }

    pub fn state(&self, key: FunctionKey) -> CompileState {
        self.functions
            .get(&key)
            .map_or(CompileState::Cold, |record| record.state)
    }

    pub fn advance_clock(&mut self, now: u64) {
        self.clock = self.clock.max(now);
    }

    pub const fn metrics(&self) -> &JitMetrics {
        &self.metrics
    }

    pub fn pin(&mut self, key: FunctionKey, tier: Tier) -> Option<ExecutionPin> {
        let artifact_key = *self.installed_keys.get(&(key, tier))?;
        self.cache.pin(artifact_key)
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn completion_sender(&self) -> CompletionSender {
        CompletionSender {
            sender: self.completion_sender.clone(),
        }
    }

    /// Applies worker completions on the caller's runtime-locked coordinator.
    pub fn drain_completions(&mut self) -> usize {
        let mut pending = Vec::new();
        let Some(receiver) = self.completion_receiver.as_ref() else {
            return 0;
        };
        while let Ok(completion) = receiver.try_recv() {
            pending.push(completion);
        }
        let drained = pending.len();
        for completion in pending {
            self.complete(completion);
        }
        drained
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
        let functions = self.functions.keys().copied().collect::<Vec<_>>();
        for key in functions {
            self.retire_state(key);
        }
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
        assert_eq!(coordinator.drain_completions(), 0);
        control.complete(CompiledArtifact::fake(Tier::Baseline));
        worker.join().unwrap();

        assert_eq!(coordinator.drain_completions(), 1);
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
        assert_eq!(coordinator.drain_completions(), 1);
        assert!(matches!(
            coordinator.state(key),
            CompileState::Backoff { attempts: 1, .. }
        ));
        assert_eq!(coordinator.metrics().compile_failures, 1);
    }
}
