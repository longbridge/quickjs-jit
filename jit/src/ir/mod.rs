//! A compact, target-independent baseline IR.

mod baseline;
mod frame_state;
mod optimized;
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
pub use types::{BinaryOp, IrOp, PollKind, StackOp, TaggedValue, UnaryOp};
