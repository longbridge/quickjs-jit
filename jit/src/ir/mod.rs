//! A compact, target-independent baseline IR.

mod baseline;
mod frame_state;
mod types;

#[cfg(feature = "test-support")]
pub(crate) use baseline::with_execution_trace;
pub(crate) use baseline::MAX_HELPER_SCRATCH_SLOTS;
pub use baseline::{BaselineIr, IrBlock, IrInstruction};
pub(crate) use frame_state::FrameStateKind;
pub use frame_state::{FrameSlot, FrameState, FrameStateId, FrameStateTable};
pub use types::{BinaryOp, IrOp, StackOp, TaggedValue, UnaryOp};
