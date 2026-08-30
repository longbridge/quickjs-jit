//! Bounded, deterministic hotness bookkeeping.

pub const BASE_CALL_THRESHOLD: u32 = 32;
pub const BASE_LOOP_THRESHOLD: u32 = 56;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotReason {
    NeutralBase,
    CallThreshold,
    LoopThreshold,
    BytecodeScale,
    HelperDensity,
    MeasuredWork,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotThresholds {
    pub calls: u32,
    pub loops: u32,
    pub rationale: HotReason,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdaptiveInputs {
    pub bytecode_bytes: u32,
    pub helper_ops: u32,
    pub instruction_count: u32,
    pub measured_work: Option<u32>,
}

impl AdaptiveInputs {
    /// A deliberately integer-only preliminary policy. Task 14 calibrates its
    /// coefficients from reproducible measurements; absent measurements the
    /// documented base thresholds remain exactly neutral.
    pub fn thresholds(self) -> HotThresholds {
        let Some(work) = self.measured_work else {
            return HotThresholds {
                calls: BASE_CALL_THRESHOLD,
                loops: BASE_LOOP_THRESHOLD,
                rationale: HotReason::NeutralBase,
            };
        };
        if work >= 4_096 {
            return HotThresholds {
                calls: BASE_CALL_THRESHOLD / 2,
                loops: BASE_LOOP_THRESHOLD / 2,
                rationale: HotReason::MeasuredWork,
            };
        }
        if self.instruction_count != 0
            && self.helper_ops.saturating_mul(4) >= self.instruction_count
        {
            return HotThresholds {
                calls: BASE_CALL_THRESHOLD.saturating_mul(2),
                loops: BASE_LOOP_THRESHOLD.saturating_mul(2),
                rationale: HotReason::HelperDensity,
            };
        }
        if self.bytecode_bytes >= 4_096 {
            return HotThresholds {
                calls: BASE_CALL_THRESHOLD.saturating_mul(2),
                loops: BASE_LOOP_THRESHOLD.saturating_mul(2),
                rationale: HotReason::BytecodeScale,
            };
        }
        HotThresholds {
            calls: BASE_CALL_THRESHOLD,
            loops: BASE_LOOP_THRESHOLD,
            rationale: HotReason::NeutralBase,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotDecision {
    Cold,
    Queue(HotReason),
    AlreadyQueued,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotnessState {
    calls: u32,
    loops: u32,
    exits: u32,
    queued: bool,
}

impl HotnessState {
    pub fn record_call_event(&mut self, count: u32) -> HotDecision {
        self.record_call_event_with_thresholds(count, AdaptiveInputs::default().thresholds())
    }

    pub fn record_loop_event(&mut self, count: u32) -> HotDecision {
        self.record_loop_event_with_thresholds(count, AdaptiveInputs::default().thresholds())
    }

    pub fn record_call_event_with_thresholds(
        &mut self,
        count: u32,
        thresholds: HotThresholds,
    ) -> HotDecision {
        self.record_calls(count);
        self.decide(self.calls >= thresholds.calls, HotReason::CallThreshold)
    }

    pub fn record_loop_event_with_thresholds(
        &mut self,
        count: u32,
        thresholds: HotThresholds,
    ) -> HotDecision {
        self.record_loops(count);
        self.decide(self.loops >= thresholds.loops, HotReason::LoopThreshold)
    }

    fn decide(&mut self, hot: bool, reason: HotReason) -> HotDecision {
        if !hot {
            HotDecision::Cold
        } else if self.mark_queued() {
            HotDecision::Queue(reason)
        } else {
            HotDecision::AlreadyQueued
        }
    }
    pub fn record_calls(&mut self, count: u32) {
        self.calls = self.calls.saturating_add(count);
    }

    pub fn record_loops(&mut self, count: u32) {
        self.loops = self.loops.saturating_add(count);
    }

    pub fn record_exits(&mut self, count: u32) {
        self.exits = self.exits.saturating_add(count);
    }

    pub const fn calls(&self) -> u32 {
        self.calls
    }

    pub const fn loops(&self) -> u32 {
        self.loops
    }

    pub const fn exits(&self) -> u32 {
        self.exits
    }

    pub fn mark_queued(&mut self) -> bool {
        if self.queued {
            false
        } else {
            self.queued = true;
            true
        }
    }

    pub fn clear_queued(&mut self) {
        self.queued = false;
    }
}
