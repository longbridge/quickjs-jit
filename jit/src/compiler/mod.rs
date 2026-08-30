//! Compiler requests and deterministic failure categories.

use crate::{code_cache::CompiledArtifact, runtime::CompileRequest};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct CompileControl {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    max_ir_bytes: usize,
}

impl CompileControl {
    pub fn new(cancelled: Arc<AtomicBool>, budget: Duration) -> Self {
        Self::with_ir_limit(cancelled, budget, usize::MAX)
    }
    pub fn with_ir_limit(
        cancelled: Arc<AtomicBool>,
        budget: Duration,
        max_ir_bytes: usize,
    ) -> Self {
        Self {
            cancelled,
            deadline: Instant::now() + budget,
            max_ir_bytes,
        }
    }
    pub fn check_ir_bytes(&self, bytes: usize) -> Result<(), CompileFailure> {
        self.check()?;
        if bytes > self.max_ir_bytes {
            Err(CompileFailure::ResourceLimit)
        } else {
            Ok(())
        }
    }
    pub fn check(&self) -> Result<(), CompileFailure> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(CompileFailure::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(CompileFailure::TimedOut)
        } else {
            Ok(())
        }
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
pub mod baseline;
#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
mod helpers;

#[cfg(any(test, feature = "test-support"))]
pub mod mock;

/// A worker-side compiler implementation.
pub trait Compiler: Send + Sync {
    fn compile(&self, request: CompileRequest) -> Result<CompiledArtifact, CompileFailure>;

    fn compile_controlled(
        &self,
        request: CompileRequest,
        control: &CompileControl,
    ) -> Result<CompiledArtifact, CompileFailure> {
        control.check()?;
        let result = self.compile(request);
        control.check()?;
        result
    }
}

/// Closed compiler failure categories used by the tiering policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileFailure {
    UnsupportedOpcode,
    Tier1Rejected(crate::bytecode::FallbackReason),
    ResourceLimit,
    TimedOut,
    Cancelled,
    CompilerPanicked,
    InvalidArtifact,
}
