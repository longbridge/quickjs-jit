use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::compiler::Compiler;

use super::{CompileRequest, Coordinator};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundCompilerError {
    InvalidLimit,
    Shutdown,
}

/// Bounded compiler workers. Jobs and completions contain owned, verified data
/// only; the owning runtime thread remains solely responsible for installation.
pub struct BackgroundCompiler {
    sender: Option<mpsc::SyncSender<CompileRequest>>,
    workers: Vec<JoinHandle<()>>,
    completion_slot: Arc<Mutex<Option<super::CompletionSender>>>,
    cancelled: Arc<AtomicBool>,
    usage: Arc<WorkerUsage>,
    max_snapshot_bytes: usize,
    max_ir_bytes: usize,
    overflow: Arc<Mutex<VecDeque<super::CompileCompletion>>>,
}

#[derive(Debug, Default)]
struct WorkerUsage {
    jobs: AtomicUsize,
    snapshots: AtomicUsize,
    ir: AtomicUsize,
}

struct UsageGuard {
    usage: Arc<WorkerUsage>,
    snapshot_bytes: usize,
    ir_bytes: usize,
}

impl Drop for UsageGuard {
    fn drop(&mut self) {
        self.usage.jobs.fetch_sub(1, Ordering::AcqRel);
        self.usage
            .snapshots
            .fetch_sub(self.snapshot_bytes, Ordering::AcqRel);
        self.usage.ir.fetch_sub(self.ir_bytes, Ordering::AcqRel);
    }
}

impl BackgroundCompiler {
    pub fn new(
        compiler: Arc<dyn Compiler>,
        worker_count: usize,
        max_pending_jobs: usize,
    ) -> Result<Self, BackgroundCompilerError> {
        Self::new_with_resource_limits(
            compiler,
            worker_count,
            max_pending_jobs,
            Duration::from_secs(30),
            usize::MAX,
            usize::MAX,
        )
    }

    pub fn new_with_resource_limits(
        compiler: Arc<dyn Compiler>,
        worker_count: usize,
        max_pending_jobs: usize,
        compile_budget: Duration,
        max_snapshot_bytes: usize,
        max_ir_bytes: usize,
    ) -> Result<Self, BackgroundCompilerError> {
        if worker_count == 0 || max_pending_jobs == 0 {
            return Err(BackgroundCompilerError::InvalidLimit);
        }
        let (sender, receiver) = mpsc::sync_channel::<CompileRequest>(max_pending_jobs);
        let receiver = Arc::new(Mutex::new(receiver));
        let completion_slot = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let usage = Arc::new(WorkerUsage::default());
        let overflow = Arc::new(Mutex::new(VecDeque::with_capacity(max_pending_jobs)));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let compiler = Arc::clone(&compiler);
            let completion_slot: Arc<Mutex<Option<super::CompletionSender>>> =
                Arc::clone(&completion_slot);
            let cancelled = Arc::clone(&cancelled);
            let worker_budget = compile_budget;
            let usage = Arc::clone(&usage);
            let overflow = Arc::clone(&overflow);
            workers.push(
                thread::Builder::new()
                    .name(format!("rquickjs-jit-{index}"))
                    .spawn(move || {
                        while let Ok(request) =
                            receiver.lock().unwrap_or_else(|p| p.into_inner()).recv()
                        {
                            let snapshot_bytes = request.snapshot().snapshot().owned_bytes();
                            let ir_bytes = snapshot_bytes.saturating_mul(32);
                            let _usage = UsageGuard {
                                usage: Arc::clone(&usage),
                                snapshot_bytes,
                                ir_bytes,
                            };
                            let sender = completion_slot
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .clone();
                            if let Some(sender) = sender {
                                let key = request.key();
                                let tier = request.tier();
                                let artifact_key = request.artifact_key();
                                let attempt_id = request.attempt_id();
                                let control = crate::compiler::CompileControl::with_ir_limit(
                                    Arc::clone(&cancelled),
                                    worker_budget,
                                    max_ir_bytes,
                                );
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        compiler.compile_controlled(request, &control)
                                    }))
                                    .unwrap_or(Err(
                                        crate::compiler::CompileFailure::CompilerPanicked,
                                    ));
                                let completion = super::CompileCompletion {
                                    key,
                                    requested_tier: tier,
                                    artifact_key,
                                    attempt_id,
                                    result,
                                };
                                if let Err(super::CompletionSendError::Full(completion)) =
                                    sender.try_send(completion)
                                {
                                    overflow
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .push_back(*completion);
                                }
                            }
                        }
                    })
                    .map_err(|_| BackgroundCompilerError::Shutdown)?,
            );
        }
        Ok(Self {
            sender: Some(sender),
            workers,
            completion_slot,
            cancelled,
            usage,
            max_snapshot_bytes,
            max_ir_bytes,
            overflow,
        })
    }

    pub fn dispatch_next(
        &mut self,
        coordinator: &mut Coordinator,
    ) -> Result<bool, BackgroundCompilerError> {
        self.drain_overflow(coordinator, super::DEFAULT_COMPLETION_DRAIN_BUDGET);
        let sender = self
            .sender
            .as_ref()
            .ok_or(BackgroundCompilerError::Shutdown)?;
        *self
            .completion_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(coordinator.completion_sender());
        let Some(request) = coordinator.begin_next() else {
            return Ok(false);
        };
        let snapshot_bytes = request.snapshot().snapshot().owned_bytes();
        let ir_bytes = snapshot_bytes.saturating_mul(32);
        if self
            .usage
            .snapshots
            .load(Ordering::Acquire)
            .saturating_add(snapshot_bytes)
            > self.max_snapshot_bytes
            || self
                .usage
                .ir
                .load(Ordering::Acquire)
                .saturating_add(ir_bytes)
                > self.max_ir_bytes
        {
            coordinator.rollback_resource_limit(request);
            coordinator.record_resource_limit_rejection();
            return Ok(false);
        }
        self.usage.jobs.fetch_add(1, Ordering::AcqRel);
        self.usage
            .snapshots
            .fetch_add(snapshot_bytes, Ordering::AcqRel);
        self.usage.ir.fetch_add(ir_bytes, Ordering::AcqRel);
        match sender.try_send(request) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(request)) => {
                self.usage.jobs.fetch_sub(1, Ordering::AcqRel);
                self.usage
                    .snapshots
                    .fetch_sub(snapshot_bytes, Ordering::AcqRel);
                self.usage.ir.fetch_sub(ir_bytes, Ordering::AcqRel);
                coordinator.rollback_dispatch(request);
                Ok(false)
            }
            Err(mpsc::TrySendError::Disconnected(request)) => {
                self.usage.jobs.fetch_sub(1, Ordering::AcqRel);
                self.usage
                    .snapshots
                    .fetch_sub(snapshot_bytes, Ordering::AcqRel);
                self.usage.ir.fetch_sub(ir_bytes, Ordering::AcqRel);
                coordinator.rollback_dispatch(request);
                Err(BackgroundCompilerError::Shutdown)
            }
        }
    }

    pub fn drain_overflow(&mut self, coordinator: &mut Coordinator, budget: usize) -> usize {
        let mut drained = 0;
        while drained < budget {
            let completion = self
                .overflow
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop_front();
            let Some(completion) = completion else { break };
            coordinator.complete(completion);
            drained += 1;
        }
        drained
    }

    pub fn live_usage(&self) -> (usize, usize, usize) {
        (
            self.usage.jobs.load(Ordering::Acquire),
            self.usage.snapshots.load(Ordering::Acquire),
            self.usage.ir.load(Ordering::Acquire),
        )
    }

    pub fn shutdown(&mut self, coordinator: &mut Coordinator) {
        self.cancelled.store(true, Ordering::Release);
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.drain_overflow(coordinator, usize::MAX);
        loop {
            let drain = coordinator.drain_completions();
            if drain.drained() == 0 {
                break;
            }
        }
    }
}
