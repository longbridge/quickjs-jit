//! A compact, target-independent baseline IR.

mod baseline;
mod frame_state;
mod inlining;
mod optimized;
mod scalar;
pub use inlining::InlineCallee;
mod types;

#[cfg(feature = "test-support")]
pub(crate) use baseline::with_execution_trace;
pub(crate) use baseline::MAX_HELPER_SCRATCH_SLOTS;
pub use baseline::{BaselineIr, IrBlock, IrInstruction};
pub(crate) use frame_state::FrameStateKind;
pub use frame_state::{FrameSlot, FrameState, FrameStateId, FrameStateTable};
pub use optimized::{
    DeoptMap, DeoptOwnership, DeoptPhase, DeoptSlot, DeoptValidationError, GuardSite,
    Materialization, MaterializedFrame, MaterializedValue, OptimizedBlock, OptimizedEffect,
    OptimizedFrameShape, OptimizedIr, OptimizedMetrics, OptimizedNode, OptimizedNodeKind,
    OwnedMaterializeError, OwnedMaterializedFrame, OwnedMaterializedValue,
    OwnershipTransitionError, SsaValueOwnership, ValueRepresentation,
};
pub use scalar::{
    ScalarBinaryOp, ScalarBitwiseOp, ScalarCall, ScalarCheck, ScalarCompareOp, ScalarFrameState,
    ScalarGraph, ScalarInlineRegion, ScalarInlineStep, ScalarNumericMode, ScalarPhiInput,
    ScalarValue, ScalarValueId,
};
pub use types::{BinaryOp, IrOp, PollKind, StackOp, TaggedValue, UnaryOp};

pub(crate) use scalar::stack_permutation;
