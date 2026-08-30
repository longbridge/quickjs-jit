use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
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
        )
    }

    pub fn new_with_resource_limits(
        compiler: Arc<dyn Compiler>,
        worker_count: usize,
        max_pending_jobs: usize,
        compile_budget: Duration,
        max_ir_bytes: usize,
    ) -> Result<Self, BackgroundCompilerError> {
        if worker_count == 0 || max_pending_jobs == 0 {
            return Err(BackgroundCompilerError::InvalidLimit);
        }
        let (sender, receiver) = mpsc::sync_channel::<CompileRequest>(max_pending_jobs);
        let receiver = Arc::new(Mutex::new(receiver));
        let completion_slot = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let compiler = Arc::clone(&compiler);
            let completion_slot: Arc<Mutex<Option<super::CompletionSender>>> =
                Arc::clone(&completion_slot);
            let cancelled = Arc::clone(&cancelled);
            let worker_budget = compile_budget;
            workers.push(
                thread::Builder::new()
                    .name(format!("rquickjs-jit-{index}"))
                    .spawn(move || {
                        while let Ok(request) =
                            receiver.lock().unwrap_or_else(|p| p.into_inner()).recv()
                        {
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
                                let _ = sender.try_send(super::CompileCompletion {
                                    key,
                                    requested_tier: tier,
                                    artifact_key,
                                    attempt_id,
                                    result,
                                });
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
        })
    }

    pub fn dispatch_next(
        &mut self,
        coordinator: &mut Coordinator,
    ) -> Result<bool, BackgroundCompilerError> {
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
        match sender.try_send(request) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(request)) => {
                coordinator.rollback_dispatch(request);
                Ok(false)
            }
            Err(mpsc::TrySendError::Disconnected(request)) => {
                coordinator.rollback_dispatch(request);
                Err(BackgroundCompilerError::Shutdown)
            }
        }
    }

    pub fn shutdown(&mut self, coordinator: &mut Coordinator) {
        self.cancelled.store(true, Ordering::Release);
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        loop {
            let drain = coordinator.drain_completions();
            if drain.drained() == 0 {
                break;
            }
        }
    }
}
