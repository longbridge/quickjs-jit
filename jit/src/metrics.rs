//! Runtime-visible JIT metrics.

/// Immutable metrics exposed by the no-op JIT backend.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JitMetrics {
    native_enabled: bool,
    pub queued: u64,
    pub compiling: u64,
    pub compile_failures: u64,
    pub stale_results: u64,
    pub installed: u64,
    pub blacklisted: u64,
    pub retired: u64,
    pub queue_saturated: u64,
    pub evicted: u64,
}

impl JitMetrics {
    pub(crate) const fn disabled() -> Self {
        Self {
            native_enabled: false,
            queued: 0,
            compiling: 0,
            compile_failures: 0,
            stale_results: 0,
            installed: 0,
            blacklisted: 0,
            retired: 0,
            queue_saturated: 0,
            evicted: 0,
        }
    }

    /// Reports whether this runtime can currently enter native JIT code.
    pub const fn native_enabled(&self) -> bool {
        self.native_enabled
    }
}
