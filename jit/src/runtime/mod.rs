//! Runtime-thread tiering coordination.

mod background;
mod coordinator;
mod hotness;
mod install;
mod invalidate;
mod osr;

pub use crate::compiler::CompileFailure;
pub use background::{BackgroundCompiler, BackgroundCompilerError};
pub use coordinator::{
    compile_and_send, ArtifactEnvironment, AttemptId, CompileCompletion, CompileRequest,
    CompileState, CompletionDrain, CompletionSendError, CompletionSender, Coordinator, FunctionKey,
    QueueError, Tier, DEFAULT_COMPLETION_DRAIN_BUDGET,
};
pub use hotness::{
    AdaptiveInputs, HotDecision, HotReason, HotThresholds, HotnessState, BASE_CALL_THRESHOLD,
    BASE_LOOP_THRESHOLD,
};
pub use osr::{OsrKey, OsrMap};

#[cfg(test)]
mod hotness_tests {
    use super::{
        AdaptiveInputs, HotDecision, HotReason, HotnessState, BASE_CALL_THRESHOLD,
        BASE_LOOP_THRESHOLD,
    };

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

    #[test]
    fn exact_default_call_and_loop_boundaries_queue_once() {
        let mut calls = HotnessState::default();
        assert_eq!(
            calls.record_call_event(BASE_CALL_THRESHOLD - 1),
            HotDecision::Cold
        );
        assert_eq!(
            calls.record_call_event(1),
            HotDecision::Queue(HotReason::CallThreshold)
        );
        assert_eq!(calls.record_call_event(1), HotDecision::AlreadyQueued);

        let mut loops = HotnessState::default();
        assert_eq!(
            loops.record_loop_event(BASE_LOOP_THRESHOLD - 1),
            HotDecision::Cold
        );
        assert_eq!(
            loops.record_loop_event(1),
            HotDecision::Queue(HotReason::LoopThreshold)
        );
        assert_eq!(loops.record_loop_event(1), HotDecision::AlreadyQueued);
    }

    #[test]
    fn eight_short_callbacks_do_not_queue_and_counters_saturate() {
        let mut hotness = HotnessState::default();
        for _ in 0..8 {
            assert_eq!(hotness.record_call_event(1), HotDecision::Cold);
        }
        hotness.record_calls(u32::MAX);
        hotness.record_loops(u32::MAX);
        assert_eq!(hotness.calls(), u32::MAX);
        assert_eq!(hotness.loops(), u32::MAX);
    }

    #[test]
    fn adaptive_thresholds_are_integer_deterministic_and_neutral_without_measurements() {
        let neutral = AdaptiveInputs::default().thresholds();
        assert_eq!(neutral.calls, BASE_CALL_THRESHOLD);
        assert_eq!(neutral.loops, BASE_LOOP_THRESHOLD);
        assert_eq!(neutral.rationale, HotReason::NeutralBase);
        assert_eq!(neutral, AdaptiveInputs::default().thresholds());
    }

    #[test]
    fn adaptive_thresholds_are_consumed_by_hotness_decisions() {
        let thresholds = AdaptiveInputs::default().thresholds();
        let mut hotness = HotnessState::default();
        assert_eq!(
            hotness.record_call_event_with_thresholds(thresholds.calls - 1, thresholds),
            HotDecision::Cold
        );
        assert_eq!(
            hotness.record_call_event_with_thresholds(1, thresholds),
            HotDecision::Queue(HotReason::CallThreshold)
        );
    }
}
