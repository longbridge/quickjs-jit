//! A compact, target-independent baseline IR.

mod baseline;
mod frame_state;
mod types;

pub use baseline::{BaselineIr, IrBlock, IrInstruction};
pub use frame_state::{FrameSlot, FrameState, FrameStateId, FrameStateTable};
pub use types::{BinaryOp, IrOp, StackOp, TaggedValue, UnaryOp};
