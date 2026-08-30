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

#[cfg(all(
    feature = "compiler",
    any(
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
        )
    )
))]
fn artifact_environment(
    runtime_id: u64,
    info: &abi::AbiInfo,
    config: &JitConfig,
    compiler: &compiler::baseline::BaselineCompiler,
) -> runtime::ArtifactEnvironment {
    fn mix(hash: u64, value: u64) -> u64 {
        (hash ^ value).wrapping_mul(0x100000001b3)
    }
    let target_identity = compiler.target_identity();
    let mut config_fingerprint = 0xcbf29ce484222325;
    for value in [
        u64::from(config.call_threshold()),
        u64::from(config.loop_threshold()),
        config.max_code_bytes() as u64,
        config.max_metadata_bytes() as u64,
        config.max_snapshot_bytes() as u64,
        config.max_ir_bytes() as u64,
        config.compile_timeout_ms(),
        config.max_queue_len() as u64,
        config.workers() as u64,
        u64::from(config.max_compile_attempts()),
    ] {
        config_fingerprint = mix(config_fingerprint, value);
    }
    runtime::ArtifactEnvironment {
        runtime_id,
        target_isa: target_identity.triple_fingerprint(),
        cpu_features: target_identity.codegen_fingerprint(),
        abi_fingerprint: mix(
            mix(info.build_fingerprint(), info.source_revision()),
            info.opcode_fingerprint(),
        ),
        config_fingerprint,
    }
}

/// Owns the guard that keeps a JIT backend attached to a runtime.
#[derive(Debug)]
pub struct Jit {
    metrics: Arc<Mutex<JitMetrics>>,
    config: JitConfig,
    _guard: rquickjs_core::runtime::RuntimeJitGuard,
    #[cfg(feature = "test-support")]
    test_environment: Option<runtime::ArtifactEnvironment>,
    #[cfg(feature = "test-support")]
    test_last_acquired_key: Arc<Mutex<Option<code_cache::ArtifactKey>>>,
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
        #[cfg(all(
            feature = "compiler",
            any(
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
                )
            )
        ))]
        let backend = ProductionBackend::new(
            runtime.jit_runtime_id(),
            &info,
            config.clone(),
            Arc::clone(&metrics),
        )?;
        #[cfg(all(
            feature = "test-support",
            feature = "compiler",
            any(
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
                )
            )
        ))]
        let test_environment = Some(backend.environment);
        #[cfg(all(
            feature = "test-support",
            feature = "compiler",
            any(
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
                )
            )
        ))]
        let test_last_acquired_key = Arc::clone(&backend.test_last_acquired_key);
        #[cfg(not(all(
            feature = "compiler",
            any(
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
                )
            )
        )))]
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
            config,
            _guard: guard,
            #[cfg(feature = "test-support")]
            test_environment: {
                #[cfg(all(
                    feature = "compiler",
                    any(
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
                        )
                    )
                ))]
                {
                    test_environment
                }
                #[cfg(not(all(
                    feature = "compiler",
                    any(
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
                        )
                    )
                )))]
                {
                    None
                }
            },
            #[cfg(feature = "test-support")]
            test_last_acquired_key: {
                #[cfg(all(
                    feature = "compiler",
                    any(
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
                        )
                    )
                ))]
                {
                    test_last_acquired_key
                }
                #[cfg(not(all(
                    feature = "compiler",
                    any(
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
                        )
                    )
                )))]
                {
                    Arc::new(Mutex::new(None))
                }
            },
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
        let snapshot = self.metrics();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.config.observe(&snapshot);
        }));
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn test_artifact_environment(&self) -> runtime::ArtifactEnvironment {
        self.test_environment.expect("production compiler backend")
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn test_last_acquired_artifact_key(&self) -> Option<code_cache::ArtifactKey> {
        *self
            .test_last_acquired_key
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(not(all(
    feature = "compiler",
    any(
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
        )
    )
)))]
#[derive(Debug)]
struct NoopBackend {
    _config: JitConfig,
}

#[cfg(not(all(
    feature = "compiler",
    any(
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
        )
    )
)))]
unsafe impl rquickjs_core::runtime::JitBackend for NoopBackend {}

#[cfg(all(
    feature = "compiler",
    any(
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
        )
    )
))]
struct ProductionBackend {
    environment: runtime::ArtifactEnvironment,
    config: JitConfig,
    coordinator: runtime::Coordinator,
    workers: runtime::BackgroundCompiler,
    requested: std::collections::HashSet<runtime::FunctionKey>,
    metrics: Arc<Mutex<JitMetrics>>,
    native_entries: u64,
    native_exits: u64,
    native_fallbacks: u64,
    native_retries: u64,
    #[cfg(feature = "test-support")]
    test_last_acquired_key: Arc<Mutex<Option<code_cache::ArtifactKey>>>,
}

#[cfg(all(
    feature = "compiler",
    any(
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
        )
    )
))]
impl ProductionBackend {
    fn new(
        runtime_id: u64,
        info: &abi::AbiInfo,
        config: JitConfig,
        metrics: Arc<Mutex<JitMetrics>>,
    ) -> Result<Self, JitError> {
        let compiler = Arc::new(compiler::baseline::BaselineCompiler::host());
        let environment = artifact_environment(runtime_id, info, &config, &compiler);
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
            environment,
        );
        coordinator.set_native_enabled(NATIVE_EXECUTION_SUPPORTED);
        Ok(Self {
            environment,
            coordinator,
            config,
            workers,
            requested: std::collections::HashSet::new(),
            metrics,
            native_entries: 0,
            native_exits: 0,
            native_fallbacks: 0,
            native_retries: 0,
            #[cfg(feature = "test-support")]
            test_last_acquired_key: Arc::new(Mutex::new(None)),
        })
    }

    fn maintenance(&mut self) {
        self.coordinator.drain_completions();
        self.workers.drain_overflow(
            &mut self.coordinator,
            runtime::DEFAULT_COMPLETION_DRAIN_BUDGET,
        );
        while matches!(self.workers.dispatch_next(&mut self.coordinator), Ok(true)) {}
        let (jobs, snapshots, ir) = self.workers.live_usage();
        self.coordinator.set_worker_usage(jobs, snapshots, ir);
        let mut snapshot = self.coordinator.metrics();
        snapshot.native_entries = self.native_entries;
        snapshot.native_exits = self.native_exits;
        snapshot.native_fallbacks = self.native_fallbacks;
        snapshot.native_retries = self.native_retries;
        *self.metrics.lock().unwrap_or_else(|p| p.into_inner()) = snapshot.clone();
    }
}

#[cfg(all(
    feature = "compiler",
    any(
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
        )
    )
))]
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
        #[cfg(feature = "test-support")]
        {
            *self
                .test_last_acquired_key
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pin.artifact().key());
        }
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

    fn native_enter(&mut self, _id: u64, _generation: u64, _pc: u32) {
        self.native_entries = self.native_entries.saturating_add(1);
    }

    fn native_exit(&mut self, _id: u64, _generation: u64, _pc: u32, exit_kind: u32) {
        self.native_exits = self.native_exits.saturating_add(1);
        if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER {
            self.native_retries = self.native_retries.saturating_add(1);
        } else if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT {
            self.native_fallbacks = self.native_fallbacks.saturating_add(1);
        }
        self.maintenance();
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

#[cfg(all(
    test,
    feature = "compiler",
    any(
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
        )
    )
))]
mod production_environment_tests {
    use super::*;

    #[test]
    fn artifact_environment_uses_runtime_abi_target_features_and_config() {
        let first = Runtime::new().unwrap();
        let second = Runtime::new().unwrap();
        let info = abi::AbiInfo::linked().unwrap();
        let compiler = compiler::baseline::BaselineCompiler::host();
        let base = artifact_environment(
            first.jit_runtime_id(),
            &info,
            &JitConfig::default(),
            &compiler,
        );
        let other_runtime = artifact_environment(
            second.jit_runtime_id(),
            &info,
            &JitConfig::default(),
            &compiler,
        );
        let other_config = artifact_environment(
            first.jit_runtime_id(),
            &info,
            &JitConfig::builder().call_threshold(99).build().unwrap(),
            &compiler,
        );
        assert_ne!(base.runtime_id, other_runtime.runtime_id);
        assert_ne!(base.runtime_id, 0);
        assert_ne!(base.target_isa, 0);
        assert_eq!(
            base.target_isa,
            compiler.target_identity().triple_fingerprint()
        );
        assert_eq!(
            base.cpu_features,
            compiler.target_identity().codegen_fingerprint()
        );
        assert_ne!(base.abi_fingerprint, 0);
        assert_ne!(base.config_fingerprint, other_config.config_fingerprint);
        assert_ne!(base.cpu_features, 0);
    }
}
