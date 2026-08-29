//! Compiler requests and deterministic failure categories.

use crate::{code_cache::CompiledArtifact, runtime::CompileRequest};

#[cfg(any(test, feature = "test-support"))]
pub mod mock;

/// A worker-side compiler implementation.
pub trait Compiler: Send + Sync {
    fn compile(&self, request: CompileRequest) -> Result<CompiledArtifact, CompileFailure>;
}

/// Closed compiler failure categories used by the tiering policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileFailure {
    UnsupportedOpcode,
    ResourceLimit,
    Cancelled,
    CompilerPanicked,
    InvalidArtifact,
}
