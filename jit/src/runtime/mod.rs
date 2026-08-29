//! Runtime-thread tiering coordination.

mod coordinator;
mod hotness;
mod install;
mod invalidate;

pub use crate::compiler::CompileFailure;
pub use coordinator::{
    compile_and_send, ArtifactEnvironment, CompileCompletion, CompileRequest, CompileState,
    CompletionSendError, CompletionSender, Coordinator, FunctionKey, QueueError, Tier,
};
pub use hotness::HotnessState;

#[cfg(test)]
mod hotness_tests {
    use super::HotnessState;

    #[test]
    fn hotness_counters_saturate_and_submission_is_single_shot() {
        let mut hotness = HotnessState::default();
        hotness.record_calls(u32::MAX);
        hotness.record_calls(1);
        hotness.record_loops(7);
        hotness.record_exits(3);

        assert_eq!(hotness.calls(), u32::MAX);
        assert_eq!(hotness.loops(), 7);
        assert_eq!(hotness.exits(), 3);
        assert!(hotness.mark_queued());
        assert!(!hotness.mark_queued());
        hotness.clear_queued();
        assert!(hotness.mark_queued());
    }
}
