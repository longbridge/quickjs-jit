//! Runtime-visible JIT metrics.

/// Immutable metrics exposed by the no-op JIT backend.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JitMetrics {
    native_enabled: bool,
    pub queued: u64,
    pub compiling: u64,
    pub compile_failures: u64,
    pub compile_timeouts: u64,
    pub stale_results: u64,
    pub installed: u64,
    pub blacklisted: u64,
    pub retired: u64,
    pub queue_saturated: u64,
    pub completion_queue_saturated: u64,
    pub worker_queue_saturated: u64,
    pub resource_limit_rejections: u64,
    pub code_bytes: usize,
    pub metadata_bytes: usize,
    pub pending_worker_jobs: usize,
    pub pending_snapshot_bytes: usize,
    pub active_ir_bytes: usize,
    pub evicted: u64,
    pub native_entries: u64,
    pub native_exits: u64,
    pub native_fallbacks: u64,
    pub native_retries: u64,
    pub osr_entries: u64,
    pub osr_not_ready: u64,
    pub osr_map_misses: u64,
    pub osr_validation_failures: u64,
}

impl JitMetrics {
    pub(crate) const fn disabled() -> Self {
        Self {
            native_enabled: false,
            queued: 0,
            compiling: 0,
            compile_failures: 0,
            compile_timeouts: 0,
            stale_results: 0,
            installed: 0,
            blacklisted: 0,
            retired: 0,
            queue_saturated: 0,
            completion_queue_saturated: 0,
            worker_queue_saturated: 0,
            resource_limit_rejections: 0,
            code_bytes: 0,
            metadata_bytes: 0,
            pending_worker_jobs: 0,
            pending_snapshot_bytes: 0,
            active_ir_bytes: 0,
            evicted: 0,
            native_entries: 0,
            native_exits: 0,
            native_fallbacks: 0,
            native_retries: 0,
            osr_entries: 0,
            osr_not_ready: 0,
            osr_map_misses: 0,
            osr_validation_failures: 0,
        }
    }

    /// Reports whether this runtime can currently enter native JIT code.
    pub const fn native_enabled(&self) -> bool {
        self.native_enabled
    }

    pub(crate) fn set_native_enabled(&mut self, enabled: bool) {
        self.native_enabled = enabled;
    }
}
