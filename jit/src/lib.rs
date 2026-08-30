//! Optional tiered JIT integration for `rquickjs`.
//!
//! The runtime attaches through a versioned engine ABI while execution remains
//! on the QuickJS interpreter until compiler tiers are enabled.

pub mod abi;
pub mod bytecode;
pub mod code_cache;
pub mod compiler;
mod config;
mod error;
#[cfg(feature = "compiler")]
pub mod ir;
mod metrics;
pub mod platform;
pub mod runtime;

#[cfg(feature = "test-support")]
#[doc(hidden)]
#[path = "../tests/support/mod.rs"]
pub mod test_support;

use core::ops::Deref;
use std::sync::{Arc, Mutex};

pub use config::{JitConfig, JitConfigBuilder, JitDiagnostic, JitDiagnosticKind};
pub use error::JitError;
pub use metrics::JitMetrics;
pub use rquickjs_core::Runtime;

const NATIVE_EXECUTION_SUPPORTED: bool = cfg!(any(
    all(
        target_os = "macos",
        target_endian = "little",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "windows",
        target_endian = "little",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "linux",
        target_endian = "little",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
));

/// Owns the guard that keeps a JIT backend attached to a runtime.
#[derive(Debug)]
pub struct Jit {
    metrics: Arc<Mutex<JitMetrics>>,
    _guard: rquickjs_core::runtime::RuntimeJitGuard,
}

impl Jit {
    /// Attaches the initial no-op backend to an existing runtime.
    pub fn attach(runtime: &Runtime, config: JitConfig) -> Result<Self, JitError> {
        let info = abi::AbiInfo::query_linked()?;
        Self::attach_with_info(runtime, config, info)
    }

    fn attach_with_info(
        runtime: &Runtime,
        config: JitConfig,
        info: abi::AbiInfo,
    ) -> Result<Self, JitError> {
        if let Err(error) = info.validate() {
            if let abi::AbiError::Incompatible(mismatch) = error {
                config.report(JitDiagnosticKind::AbiMismatch(mismatch));
            }
            return Err(error.into());
        }

        let metrics = Arc::new(Mutex::new(JitMetrics::disabled()));
        config.observe(&metrics.lock().unwrap_or_else(|p| p.into_inner()));
        #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
        let backend = ProductionBackend::new(config.clone(), Arc::clone(&metrics))?;
        #[cfg(not(all(feature = "compiler", not(target_family = "wasm"))))]
        let backend = NoopBackend {
            _config: config.clone(),
        };
        let guard = match runtime.attach_jit_backend(backend) {
            Ok(guard) => guard,
            Err(error) => {
                config.report(JitDiagnosticKind::BackendAttachment);
                return Err(error.into());
            }
        };
        Ok(Self {
            metrics,
            _guard: guard,
        })
    }

    /// Fails when the current target cannot support native execution.
    pub fn require_native(&self) -> Result<(), JitError> {
        if NATIVE_EXECUTION_SUPPORTED {
            Ok(())
        } else {
            Err(JitError::UnsupportedPlatform)
        }
    }

    /// Returns the metrics associated with this backend guard.
    pub fn metrics(&self) -> JitMetrics {
        self.metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Performs bounded installation and reclamation work on the runtime thread.
    pub fn poll(&self) {
        self._guard.poll();
    }
}

#[cfg(not(all(feature = "compiler", not(target_family = "wasm"))))]
#[derive(Debug)]
struct NoopBackend {
    _config: JitConfig,
}

#[cfg(not(all(feature = "compiler", not(target_family = "wasm"))))]
unsafe impl rquickjs_core::runtime::JitBackend for NoopBackend {}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
struct ProductionBackend {
    config: JitConfig,
    coordinator: runtime::Coordinator,
    workers: runtime::BackgroundCompiler,
    requested: std::collections::HashSet<runtime::FunctionKey>,
    metrics: Arc<Mutex<JitMetrics>>,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
impl ProductionBackend {
    fn new(config: JitConfig, metrics: Arc<Mutex<JitMetrics>>) -> Result<Self, JitError> {
        let compiler = Arc::new(compiler::baseline::BaselineCompiler::host());
        let workers = runtime::BackgroundCompiler::new_with_resource_limits(
            compiler,
            config.workers(),
            config.max_queue_len(),
            std::time::Duration::from_millis(config.compile_timeout_ms()),
            config.max_snapshot_bytes(),
            config.max_ir_bytes(),
        )
        .map_err(|_| JitError::InvalidConfig("workers"))?;
        let mut coordinator = runtime::Coordinator::with_environment_and_metadata_limit(
            config.max_queue_len(),
            config.max_queue_len(),
            config.max_compile_attempts(),
            config.max_code_bytes(),
            config.max_metadata_bytes(),
            runtime::ArtifactEnvironment::default(),
        );
        coordinator.set_native_enabled(NATIVE_EXECUTION_SUPPORTED);
        Ok(Self {
            coordinator,
            config,
            workers,
            requested: std::collections::HashSet::new(),
            metrics,
        })
    }

    fn maintenance(&mut self) {
        self.coordinator.drain_completions();
        while matches!(self.workers.dispatch_next(&mut self.coordinator), Ok(true)) {}
        let (jobs, snapshots, ir) = self.workers.live_usage();
        self.coordinator.set_worker_usage(jobs, snapshots, ir);
        let snapshot = self.coordinator.metrics();
        *self.metrics.lock().unwrap_or_else(|p| p.into_inner()) = snapshot.clone();
        self.config.observe(&snapshot);
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
unsafe impl rquickjs_core::runtime::JitBackend for ProductionBackend {
    fn poll(&mut self) {
        self.maintenance();
    }

    fn record_hot(&mut self, event: &rquickjs_core::qjs::JSJitHotEvent) -> u32 {
        self.maintenance();
        if event.pc != 0 || event.kind != rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_CALL {
            return 0;
        }
        let key = runtime::FunctionKey::new(event.function.id, event.function.generation);
        u32::from(self.requested.insert(key))
    }

    fn submit_snapshot(&mut self, snapshot: *mut rquickjs_core::qjs::JSJitFunctionSnapshot) {
        struct FreeSnapshot(*mut rquickjs_core::qjs::JSJitFunctionSnapshot);
        impl Drop for FreeSnapshot {
            fn drop(&mut self) {
                unsafe { rquickjs_core::qjs::JS_JitFreeSnapshot(self.0) };
            }
        }
        let Some(raw) = std::ptr::NonNull::new(snapshot) else {
            return;
        };
        let _free = FreeSnapshot(snapshot);
        let copied = unsafe { bytecode::CompileSnapshot::copy_borrowed_raw(raw.as_ref()) };
        let Ok(snapshot) = copied else { return };
        if snapshot.owned_bytes() > self.config.max_snapshot_bytes() {
            self.coordinator.record_resource_limit_rejection();
            self.maintenance();
            return;
        }
        let key = runtime::FunctionKey::new(snapshot.function_id(), snapshot.generation());
        let Ok(verified) = snapshot.verify(bytecode::VerifyLimits::default()) else {
            return;
        };
        let _ = self
            .coordinator
            .queue(key, runtime::Tier::Baseline, verified);
        self.maintenance();
    }

    fn acquire_entry(
        &mut self,
        id: u64,
        generation: u64,
        pc: u32,
    ) -> rquickjs_core::qjs::JSJitEntryHandle {
        let mut empty = rquickjs_core::qjs::JSJitEntryHandle {
            struct_size: core::mem::size_of::<rquickjs_core::qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: None,
            pin: core::ptr::null_mut(),
            stack_map_count: 0,
            helper_abi_version: 0,
        };
        self.maintenance();
        if pc != 0 {
            return empty;
        }
        let Some(pin) = self.coordinator.pin(
            runtime::FunctionKey::new(id, generation),
            runtime::Tier::Baseline,
        ) else {
            return empty;
        };
        let Some(published) = pin.artifact().published() else {
            return empty;
        };
        let entry = published.as_ptr();
        let stack_map_count = u32::try_from(published.stack_maps().len()).unwrap_or(u32::MAX);
        let pin = Box::into_raw(Box::new(pin)).cast();
        empty.entry = Some(unsafe {
            core::mem::transmute::<
                *const u8,
                unsafe extern "C" fn(
                    *mut rquickjs_core::qjs::JSJitExecFrame,
                ) -> rquickjs_core::qjs::JSJitExit,
            >(entry)
        });
        empty.pin = pin;
        empty.stack_map_count = stack_map_count;
        empty.helper_abi_version = rquickjs_core::qjs::QJSJIT_HELPER_ABI_VERSION;
        empty
    }

    fn release_entry(&mut self, entry: rquickjs_core::qjs::JSJitEntryHandle) {
        if !entry.pin.is_null() {
            unsafe { drop(Box::from_raw(entry.pin.cast::<code_cache::ExecutionPin>())) };
        }
    }

    fn function_retire(&mut self, id: u64, generation: u64) {
        let key = runtime::FunctionKey::new(id, generation);
        self.requested.remove(&key);
        self.coordinator.retire(key);
        self.maintenance();
    }

    fn runtime_detach(&mut self) {
        self.workers.shutdown(&mut self.coordinator);
        self.coordinator.shutdown();
        self.maintenance();
    }

    fn memory_used(&self) -> usize {
        self.coordinator.cache_bytes()
    }
}

/// Builder for an owning [`JitRuntime`].
#[derive(Clone, Debug, Default)]
pub struct JitRuntimeBuilder {
    config: JitConfig,
}

impl JitRuntimeBuilder {
    /// Sets the JIT policy and resource limits.
    pub fn config(mut self, config: JitConfig) -> Self {
        self.config = config;
        self
    }

    /// Constructs an interpreter runtime with a disabled JIT guard.
    pub fn build(self) -> Result<JitRuntime, JitError> {
        let runtime = Runtime::new()?;
        let jit = Jit::attach(&runtime, self.config)?;
        Ok(JitRuntime { runtime, jit })
    }
}

/// An owning QuickJS runtime paired with its JIT guard.
pub struct JitRuntime {
    runtime: Runtime,
    jit: Jit,
}

impl JitRuntime {
    /// Starts building an owning JIT runtime.
    pub fn builder() -> JitRuntimeBuilder {
        JitRuntimeBuilder::default()
    }

    /// Returns the disabled JIT guard.
    pub const fn jit(&self) -> &Jit {
        &self.jit
    }

    /// Returns runtime metrics.
    pub fn metrics(&self) -> JitMetrics {
        self.jit.metrics()
    }
}

impl Deref for JitRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
