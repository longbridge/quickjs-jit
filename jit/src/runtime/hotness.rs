//! Bounded hotness bookkeeping. Threshold policy is added with OSR support.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotnessState {
    calls: u32,
    loops: u32,
    exits: u32,
    queued: bool,
}

impl HotnessState {
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
