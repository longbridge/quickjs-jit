//! Optional tiered JIT integration for `rquickjs`.
//!
//! The runtime attaches through a versioned engine ABI while execution remains
//! on the QuickJS interpreter until compiler tiers are enabled.

pub mod abi;
pub mod bytecode;
pub mod code_cache;
pub mod compiler;
mod config;
pub mod correctness;
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
#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use config::{JitConfig, JitConfigBuilder, JitDiagnostic, JitDiagnosticKind, JitTierPolicy};
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
        match config.tier_policy() {
            JitTierPolicy::Automatic => 0,
            JitTierPolicy::BaselineOnly => 1,
            JitTierPolicy::Optimize => 2,
        },
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
    hotness: std::collections::HashMap<runtime::FunctionKey, runtime::HotnessState>,
    optimizing_requested: std::collections::HashSet<runtime::FunctionKey>,
    optimizing_hotness: std::collections::HashMap<runtime::FunctionKey, runtime::HotnessState>,
    optimizing_snapshots:
        std::collections::HashMap<runtime::FunctionKey, bytecode::VerifiedFunction>,
    tier2_sources: std::collections::HashMap<runtime::FunctionKey, bytecode::VerifiedFunction>,
    feedback: runtime::FeedbackTable,
    metrics: Arc<Mutex<JitMetrics>>,
    native_entries: u64,
    native_exits: u64,
    native_fallbacks: u64,
    native_retries: u64,
    osr_entries: u64,
    osr_not_ready: u64,
    osr_map_misses: u64,
    osr_validation_failures: u64,
    osr_attempts: u64,
    osr_generated_retries: u64,
    osr_validation: Arc<OsrValidationMetrics>,
    clock: u64,
    queue_reasons: std::collections::HashMap<runtime::FunctionKey, runtime::HotReason>,
    prequeue_backoff: std::collections::HashMap<runtime::FunctionKey, (u8, u64)>,
    hot_call_queues: u64,
    hot_loop_queues: u64,
    adaptive_neutral_queues: u64,
    adaptive_inputs_recorded: u64,
    snapshot_requests: u64,
    stable_path_compile_requests: u64,
    pending_entry_tiers:
        std::collections::HashMap<runtime::FunctionKey, std::collections::VecDeque<runtime::Tier>>,
    execution_starts:
        std::collections::HashMap<runtime::FunctionKey, Vec<(std::time::Instant, runtime::Tier)>>,
    execution_profiles: std::collections::HashMap<runtime::FunctionKey, ProductionProfile>,
    profitability_evaluations: u64,
    profitability_approved: u64,
    profitability_rejected: u64,
    profitability_backoff: std::collections::HashMap<runtime::FunctionKey, (u8, u64)>,
    profitability_blacklisted: std::collections::HashSet<runtime::FunctionKey>,
    benefit_recordings: u64,
    measured_benefit_ns: u64,
    compiler_measurements: Arc<CompilerMeasurements>,
    install_ns: u64,
    peak_compiler_bytes: usize,
    #[cfg(feature = "test-support")]
    test_last_acquired_key: Arc<Mutex<Option<code_cache::ArtifactKey>>>,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
#[derive(Default)]
struct CompilerMeasurements {
    elapsed_ns: AtomicU64,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
struct MeasuredCompiler<C> {
    inner: C,
    measurements: Arc<CompilerMeasurements>,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
impl<C: compiler::Compiler> compiler::Compiler for MeasuredCompiler<C> {
    fn compile(
        &self,
        request: runtime::CompileRequest,
    ) -> Result<code_cache::CompiledArtifact, compiler::CompileFailure> {
        let started = std::time::Instant::now();
        let result = self.inner.compile(request);
        self.measurements.elapsed_ns.fetch_add(
            started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        result
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
#[derive(Clone, Copy, Debug, Default)]
struct ProductionProfile {
    bytecodes: u64,
    helper_calls: u64,
    baseline_executions: u64,
    baseline_ns: u64,
    optimized_executions: u64,
    optimized_ns: u64,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
struct ProductionEntryPin {
    execution: code_cache::ExecutionPin,
    native: *const u8,
    runtime_id: u64,
    key: runtime::FunctionKey,
    pc: u32,
    stack_map_count: u32,
    osr: Option<runtime::OsrMap>,
    deopt_sites: Box<[(ir::OptimizedFrameShape, ir::DeoptMap)]>,
    validation: Arc<OsrValidationMetrics>,
    #[cfg(feature = "test-support")]
    stress_gc: bool,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
type PendingDeoptGuards = std::collections::HashMap<
    (u64, u64),
    std::collections::VecDeque<(u32, Option<runtime::ObservedType>)>,
>;

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
#[derive(Default)]
struct OsrValidationMetrics {
    successes: AtomicU64,
    failures: AtomicU64,
    validation_retries: Mutex<std::collections::HashMap<(u64, u64, u32), u64>>,
    deopt_guards: Mutex<PendingDeoptGuards>,
    deopt_materializations: AtomicU64,
    side_path_entries: AtomicU64,
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
impl OsrValidationMetrics {
    fn mark_validation_retry(&self, id: u64, generation: u64, pc: u32) {
        let mut retries = self
            .validation_retries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = retries.entry((id, generation, pc)).or_default();
        *pending = pending.saturating_add(1);
    }

    fn take_validation_retry(&self, id: u64, generation: u64, pc: u32) -> bool {
        let mut retries = self
            .validation_retries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (id, generation, pc);
        let Some(pending) = retries.get_mut(&key) else {
            return false;
        };
        *pending -= 1;
        if *pending == 0 {
            retries.remove(&key);
        }
        true
    }

    fn mark_deopt_guard(
        &self,
        id: u64,
        generation: u64,
        guard: u32,
        observed: Option<runtime::ObservedType>,
    ) {
        self.deopt_guards
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry((id, generation))
            .or_default()
            .push_back((guard, observed));
    }

    fn take_deopt_guard(
        &self,
        id: u64,
        generation: u64,
    ) -> Option<(u32, Option<runtime::ObservedType>)> {
        let mut guards = self
            .deopt_guards
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let queue = guards.get_mut(&(id, generation))?;
        let guard = queue.pop_front();
        if queue.is_empty() {
            guards.remove(&(id, generation));
        }
        guard
    }

    fn take_side_path_entries(&self) -> u64 {
        self.side_path_entries.swap(0, Ordering::AcqRel)
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn record_osr_validation(metrics: &OsrValidationMetrics, pc: u32, valid: bool) {
    if pc == 0 {
        return;
    }
    if valid {
        metrics.successes.fetch_add(1, Ordering::Relaxed);
    } else {
        metrics.failures.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
const fn is_generated_osr_retry(pc: u32, exit_kind: u32, validation_retry: bool) -> bool {
    pc != 0
        && exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER
        && !validation_retry
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn apply_stress_gc_after_validation(
    frame: &mut rquickjs_core::qjs::JSJitExecFrame,
    stress_gc: bool,
    valid: bool,
) {
    if valid && stress_gc {
        frame.flags |= rquickjs_core::qjs::JS_JIT_FRAME_STRESS_GC;
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn retry_exit() -> rquickjs_core::qjs::JSJitExit {
    rquickjs_core::qjs::JSJitExit {
        kind: rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
        reserved: 0,
        resume_pc: core::ptr::null(),
        resume_stack_top: core::ptr::null_mut(),
    }
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn validate_production_frame(
    frame: &rquickjs_core::qjs::JSJitExecFrame,
    runtime_id: u64,
    key: runtime::FunctionKey,
    pc: u32,
    stack_map_count: u32,
    osr: Option<&runtime::OsrMap>,
) -> bool {
    use bytecode::SlotKind;
    use rquickjs_core::qjs;

    let expected_pc_address = (frame.bytecode_start as usize).checked_add(pc as usize);
    if frame.struct_size != core::mem::size_of::<qjs::JSJitExecFrame>() as u32
        || frame.rt.is_null()
        || frame.ctx.is_null()
        || frame.runtime_api.is_null()
        || frame.runtime_id != runtime_id
        || frame.frame_cookie == 0
        || frame.function_id != key.id
        || frame.generation != key.generation
        || frame.bytecode_start.is_null()
        || frame.pc.is_null()
        || Some(frame.pc as usize) != expected_pc_address
        || frame.entry.stack_map_count != stack_map_count
        || frame.entry.helper_abi_version != qjs::QJSJIT_HELPER_ABI_VERSION
        || frame.stack_base.is_null()
        || frame.stack_top.is_null()
        || frame.stack_capacity.is_null()
        || (frame.stack_base as usize) > (frame.stack_top as usize)
        || (frame.stack_top as usize) > (frame.stack_capacity as usize)
    {
        return false;
    }
    let Some(map) = osr else {
        return pc == 0;
    };
    let value_size = core::mem::size_of::<qjs::JSValue>();
    let expected_top = (frame.stack_base as usize)
        .checked_add(usize::from(map.stack_depth()).saturating_mul(value_size));
    if expected_top != Some(frame.stack_top as usize)
        || frame.arg_buf.is_null()
        || frame.var_buf.is_null()
        || map.live_slots().len()
            != usize::from(map.argument_count())
                + usize::from(map.local_count())
                + usize::from(map.stack_depth())
    {
        return false;
    }
    for (index, kind) in map.live_slots().iter().copied().enumerate() {
        let value = if index < usize::from(map.argument_count()) {
            unsafe { &*frame.arg_buf.add(index) }
        } else if index < usize::from(map.argument_count()) + usize::from(map.local_count()) {
            unsafe { &*frame.var_buf.add(index - usize::from(map.argument_count())) }
        } else {
            unsafe {
                &*frame
                    .stack_base
                    .add(index - usize::from(map.argument_count()) - usize::from(map.local_count()))
            }
        };
        let valid = match kind {
            SlotKind::Tagged => true,
            SlotKind::Int32 | SlotKind::CatchOffset => value.tag == i64::from(qjs::JS_TAG_INT),
            SlotKind::Float64 => value.tag == i64::from(qjs::JS_TAG_FLOAT64),
            SlotKind::Uninitialized => value.tag == i64::from(qjs::JS_TAG_UNINITIALIZED),
        };
        if !valid {
            return false;
        }
    }
    true
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
unsafe extern "C" fn production_entry_trampoline(
    frame: *mut rquickjs_core::qjs::JSJitExecFrame,
) -> rquickjs_core::qjs::JSJitExit {
    use rquickjs_core::qjs;

    if frame.is_null() {
        return retry_exit();
    }
    let frame = unsafe { &mut *frame };
    if frame.entry.pin.is_null() {
        return retry_exit();
    }
    let pin = unsafe { &*frame.entry.pin.cast::<ProductionEntryPin>() };
    let _keep_execution_pinned = &pin.execution;
    let valid = validate_production_frame(
        frame,
        pin.runtime_id,
        pin.key,
        pin.pc,
        pin.stack_map_count,
        pin.osr.as_ref(),
    );
    record_osr_validation(&pin.validation, pin.pc, valid);
    if !valid {
        pin.validation
            .mark_validation_retry(pin.key.id, pin.key.generation, pin.pc);
        return retry_exit();
    }
    #[cfg(feature = "test-support")]
    apply_stress_gc_after_validation(frame, pin.stress_gc, valid);
    type NativeEntry = unsafe extern "C" fn(*mut qjs::JSJitExecFrame) -> qjs::JSJitExit;
    let native = unsafe { core::mem::transmute::<*const u8, NativeEntry>(pin.native) };
    let mut exit = unsafe { native(frame as *const _ as *mut _) };
    if frame.flags & qjs::JS_JIT_FRAME_SIDE_PATH_HIT != 0 {
        frame.flags &= !qjs::JS_JIT_FRAME_SIDE_PATH_HIT;
        pin.validation
            .side_path_entries
            .fetch_add(1, Ordering::Release);
    }
    if exit.kind != qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT {
        return exit;
    }
    let Some(guard) = exit.reserved.checked_sub(1) else {
        return retry_exit();
    };
    let Some(resume_pc) = (exit.resume_pc as usize)
        .checked_sub(frame.bytecode_start as usize)
        .and_then(|pc| u32::try_from(pc).ok())
    else {
        return retry_exit();
    };
    let Some((shape, map)) = pin.deopt_sites.iter().find(|(shape, map)| {
        map.guard() == guard && map.resume_pc() == resume_pc && map.validate(*shape).is_ok()
    }) else {
        return retry_exit();
    };
    /* Narrow Tier 2 currently emits identity recipes only. Execute their
     * two-phase contract now: validate the complete transaction first, then
     * commit the already-materialized frame by publishing the resume state.
     * Any future non-identity/stack recipe fails closed until it has an owning
     * duplication implementation. */
    if map.validate_identity_materialization(*shape).is_err() {
        return retry_exit();
    }
    pin.validation
        .deopt_materializations
        .fetch_add(1, Ordering::Relaxed);
    pin.validation.mark_deopt_guard(
        pin.key.id,
        pin.key.generation,
        guard,
        observed_deopt_type(frame, *shape),
    );
    /* The one-based identity is internal to the pinned backend artifact. C's
     * stable ABI keeps this field reserved and receives only validated zero. */
    exit.reserved = 0;
    exit
}

#[cfg(all(feature = "compiler", not(target_family = "wasm")))]
fn observed_deopt_type(
    frame: &rquickjs_core::qjs::JSJitExecFrame,
    shape: ir::OptimizedFrameShape,
) -> Option<runtime::ObservedType> {
    use rquickjs_core::qjs;
    let values = unsafe {
        core::slice::from_raw_parts(frame.arg_buf, usize::from(shape.arguments()))
            .iter()
            .chain(core::slice::from_raw_parts(
                frame.var_buf,
                usize::from(shape.locals()),
            ))
    };
    values
        .filter_map(|value| {
            let tag = unsafe { qjs::JS_VALUE_GET_TAG(*value) };
            Some(match tag {
                qjs::JS_TAG_INT => runtime::ObservedType::Int32,
                qjs::JS_TAG_FLOAT64 => runtime::ObservedType::Float64,
                qjs::JS_TAG_BOOL => runtime::ObservedType::Bool,
                qjs::JS_TAG_NULL => runtime::ObservedType::Null,
                qjs::JS_TAG_UNDEFINED => runtime::ObservedType::Undefined,
                qjs::JS_TAG_STRING => runtime::ObservedType::String,
                qjs::JS_TAG_OBJECT => runtime::ObservedType::Object,
                _ => return None,
            })
        })
        .find(|observed| *observed != runtime::ObservedType::Int32)
}

#[cfg(all(test, feature = "compiler", not(target_family = "wasm")))]
mod production_osr_validation_tests {
    use super::*;
    use rquickjs_core::qjs;

    fn bytes(frame: &qjs::JSJitExecFrame) -> Vec<u8> {
        unsafe {
            core::slice::from_raw_parts(
                (frame as *const qjs::JSJitExecFrame).cast::<u8>(),
                core::mem::size_of::<qjs::JSJitExecFrame>(),
            )
            .to_vec()
        }
    }

    #[test]
    fn every_invalid_osr_field_retries_without_mutating_the_frame() {
        let mut bytecode = [0_u8; 8];
        let mut stack = [unsafe { core::mem::zeroed::<qjs::JSValue>() }; 2];
        stack[0].tag = i64::from(qjs::JS_TAG_INT);
        let dangling_value = core::ptr::NonNull::<qjs::JSValue>::dangling().as_ptr();
        let key = runtime::FunctionKey::new(7, 3);
        let map = runtime::OsrMap::new(
            runtime::OsrKey::new(key, 4),
            4,
            1,
            vec![bytecode::SlotKind::Int32],
        );
        let mut frame: qjs::JSJitExecFrame = unsafe { core::mem::zeroed() };
        frame.struct_size = core::mem::size_of::<qjs::JSJitExecFrame>() as u32;
        frame.rt = core::ptr::NonNull::dangling().as_ptr();
        frame.ctx = core::ptr::NonNull::dangling().as_ptr();
        frame.function_id = key.id;
        frame.generation = key.generation;
        frame.arg_buf = dangling_value;
        frame.var_buf = dangling_value;
        frame.stack_base = stack.as_mut_ptr();
        frame.stack_top = unsafe { stack.as_mut_ptr().add(1) };
        frame.stack_capacity = unsafe { stack.as_mut_ptr().add(2) };
        frame.bytecode_start = bytecode.as_mut_ptr();
        frame.pc = unsafe { bytecode.as_mut_ptr().add(4) };
        frame.runtime_api = core::ptr::NonNull::dangling().as_ptr();
        frame.runtime_id = 11;
        frame.frame_cookie = 9;
        frame.entry.stack_map_count = 5;
        frame.entry.helper_abi_version = qjs::QJSJIT_HELPER_ABI_VERSION;
        assert!(validate_production_frame(&frame, 11, key, 4, 5, Some(&map)));

        let mut invalid = Vec::new();
        let mut value = frame;
        value.runtime_id = 12;
        invalid.push(value);
        let mut value = frame;
        value.function_id = 8;
        invalid.push(value);
        let mut value = frame;
        value.generation = 4;
        invalid.push(value);
        let mut value = frame;
        value.pc = bytecode.as_mut_ptr();
        invalid.push(value);
        let mut value = frame;
        value.frame_cookie = 0;
        invalid.push(value);
        let mut value = frame;
        value.entry.stack_map_count = 4;
        invalid.push(value);
        let mut value = frame;
        value.entry.helper_abi_version = 0;
        invalid.push(value);
        let mut value = frame;
        value.stack_top = value.stack_base;
        invalid.push(value);

        for candidate in &invalid {
            let before = bytes(candidate);
            assert!(!validate_production_frame(
                candidate,
                11,
                key,
                4,
                5,
                Some(&map)
            ));
            assert_eq!(bytes(candidate), before);
        }
        let before = bytes(&frame);
        stack[0].tag = i64::from(qjs::JS_TAG_UNDEFINED);
        let tag_before = stack[0].tag;
        assert!(!validate_production_frame(
            &frame,
            11,
            key,
            4,
            5,
            Some(&map)
        ));
        assert_eq!(
            bytes(&frame),
            before,
            "slot rejection does not mutate frame pointers"
        );
        assert_eq!(stack[0].tag, tag_before, "slot owner/refcount is untouched");
    }

    #[test]
    fn osr_success_is_counted_only_after_frame_validation() {
        let metrics = OsrValidationMetrics::default();
        record_osr_validation(&metrics, 9, false);
        assert_eq!(metrics.successes.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.failures.load(Ordering::Relaxed), 1);
        assert!(is_generated_osr_retry(
            9,
            qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
            false,
        ));
        assert!(!is_generated_osr_retry(
            0,
            qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
            false,
        ));
        assert!(!is_generated_osr_retry(
            9,
            qjs::JSJitExitKind_JS_JIT_EXIT_DONE,
            false,
        ));
        assert!(!is_generated_osr_retry(
            9,
            qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
            true,
        ));
        record_osr_validation(&metrics, 9, true);
        assert_eq!(metrics.successes.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.failures.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn invalid_stress_frame_is_byte_identical_and_does_not_arm_stress_gc() {
        let mut frame: qjs::JSJitExecFrame = unsafe { core::mem::zeroed() };
        let before = bytes(&frame);
        apply_stress_gc_after_validation(&mut frame, true, false);
        assert_eq!(bytes(&frame), before);
    }

    #[test]
    fn validation_retry_is_not_attributed_to_generated_native_code() {
        let metrics = OsrValidationMetrics::default();
        metrics.mark_validation_retry(7, 3, 9);
        assert!(metrics.take_validation_retry(7, 3, 9));
        assert!(!metrics.take_validation_retry(7, 3, 9));
        assert!(!metrics.take_validation_retry(7, 3, 10));
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
impl ProductionBackend {
    fn new(
        runtime_id: u64,
        info: &abi::AbiInfo,
        config: JitConfig,
        metrics: Arc<Mutex<JitMetrics>>,
    ) -> Result<Self, JitError> {
        let identity_compiler = compiler::baseline::BaselineCompiler::host();
        let environment = artifact_environment(runtime_id, info, &config, &identity_compiler);
        let compiler_measurements = Arc::new(CompilerMeasurements::default());
        let compiler = Arc::new(MeasuredCompiler {
            inner: compiler::optimized::TieredCompiler::host(),
            measurements: Arc::clone(&compiler_measurements),
        });
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
        let feedback_capacity = config.max_queue_len().max(32);
        Ok(Self {
            environment,
            coordinator,
            config,
            workers,
            requested: std::collections::HashSet::new(),
            hotness: std::collections::HashMap::new(),
            optimizing_requested: std::collections::HashSet::new(),
            optimizing_hotness: std::collections::HashMap::new(),
            optimizing_snapshots: std::collections::HashMap::new(),
            tier2_sources: std::collections::HashMap::new(),
            feedback: runtime::FeedbackTable::new(feedback_capacity, 3),
            metrics,
            native_entries: 0,
            native_exits: 0,
            native_fallbacks: 0,
            native_retries: 0,
            osr_entries: 0,
            osr_not_ready: 0,
            osr_map_misses: 0,
            osr_validation_failures: 0,
            osr_attempts: 0,
            osr_generated_retries: 0,
            osr_validation: Arc::new(OsrValidationMetrics::default()),
            clock: 0,
            queue_reasons: std::collections::HashMap::new(),
            prequeue_backoff: std::collections::HashMap::new(),
            hot_call_queues: 0,
            hot_loop_queues: 0,
            adaptive_neutral_queues: 0,
            adaptive_inputs_recorded: 0,
            snapshot_requests: 0,
            stable_path_compile_requests: 0,
            execution_starts: std::collections::HashMap::new(),
            pending_entry_tiers: std::collections::HashMap::new(),
            execution_profiles: std::collections::HashMap::new(),
            profitability_evaluations: 0,
            profitability_approved: 0,
            profitability_rejected: 0,
            profitability_backoff: std::collections::HashMap::new(),
            profitability_blacklisted: std::collections::HashSet::new(),
            benefit_recordings: 0,
            measured_benefit_ns: 0,
            compiler_measurements,
            install_ns: 0,
            peak_compiler_bytes: 0,
            #[cfg(feature = "test-support")]
            test_last_acquired_key: Arc::new(Mutex::new(None)),
        })
    }

    fn maintenance(&mut self) {
        self.clock = self.clock.saturating_add(1);
        self.coordinator.advance_clock(self.clock);
        let installed_before = self.coordinator.metrics().installed;
        let install_started = std::time::Instant::now();
        self.coordinator.drain_completions();
        if self.coordinator.metrics().installed > installed_before {
            self.install_ns = self.install_ns.saturating_add(
                install_started
                    .elapsed()
                    .as_nanos()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
        }
        let ready_for_tier2 = self
            .optimizing_snapshots
            .keys()
            .copied()
            .filter(|key| {
                if self.profitability_blacklisted.contains(key) {
                    return false;
                }
                if self
                    .profitability_backoff
                    .get(key)
                    .is_some_and(|(_, retry_at)| self.clock < *retry_at)
                {
                    return false;
                }
                matches!(
                    self.coordinator.tier_state(*key, runtime::Tier::Baseline),
                    runtime::CompileState::Installed(_)
                ) && matches!(
                    self.coordinator.tier_state(*key, runtime::Tier::Optimizing),
                    runtime::CompileState::Cold
                )
            })
            .collect::<Vec<_>>();
        for key in if self.config.tier_policy() == JitTierPolicy::BaselineOnly {
            Vec::new()
        } else {
            ready_for_tier2
        } {
            #[cfg(feature = "test-support")]
            let forced = self.config.force_optimized();
            #[cfg(not(feature = "test-support"))]
            let forced = false;
            if !forced {
                let measured = self
                    .execution_profiles
                    .get(&key)
                    .copied()
                    .unwrap_or_default();
                let installed = self.coordinator.metrics().installed.max(1);
                let profile = runtime::Profile {
                    bytecodes: measured.bytecodes,
                    helper_calls: measured.helper_calls,
                    compile_ns: self
                        .compiler_measurements
                        .elapsed_ns
                        .load(Ordering::Relaxed)
                        / installed,
                    install_ns: self.install_ns / installed,
                    executions: measured.baseline_executions,
                    baseline_ns: measured.baseline_ns,
                    code_bytes: measured.bytecodes.saturating_mul(8),
                    ..runtime::Profile::default()
                };
                self.profitability_evaluations = self.profitability_evaluations.saturating_add(1);
                if runtime::Profitability::default()
                    .evaluate_trial(profile)
                    .tier
                    != runtime::Decision::Optimize
                {
                    self.profitability_rejected = self.profitability_rejected.saturating_add(1);
                    let entry = self
                        .profitability_backoff
                        .entry(key)
                        .or_insert((0, self.clock));
                    entry.0 = entry.0.saturating_add(1);
                    if entry.0 >= 5 {
                        self.profitability_blacklisted.insert(key);
                    } else {
                        entry.1 = self.clock.saturating_add(1u64 << entry.0.min(20));
                    }
                    continue;
                }
                self.profitability_backoff.remove(&key);
                self.profitability_approved = self.profitability_approved.saturating_add(1);
            }
            if let Some(snapshot) = self.optimizing_snapshots.remove(&key) {
                #[cfg(feature = "test-support")]
                let feedback = if self.config.force_optimized() {
                    let mut deterministic = runtime::FeedbackTable::new(1, 1);
                    deterministic.observe_type(
                        key,
                        0,
                        runtime::FeedbackKind::Value,
                        runtime::ObservedType::Int32,
                    );
                    deterministic.snapshot(self.clock.max(1))
                } else {
                    self.feedback.snapshot(self.clock)
                };
                #[cfg(not(feature = "test-support"))]
                let feedback = self.feedback.snapshot(self.clock);
                if self
                    .coordinator
                    .queue_with_feedback(key, runtime::Tier::Optimizing, snapshot.clone(), feedback)
                    .is_err()
                {
                    self.optimizing_snapshots.insert(key, snapshot);
                } else {
                    self.tier2_sources.insert(key, snapshot);
                }
            }
        }
        self.workers.drain_overflow(
            &mut self.coordinator,
            runtime::DEFAULT_COMPLETION_DRAIN_BUDGET,
        );
        while matches!(self.workers.dispatch_next(&mut self.coordinator), Ok(true)) {}
        let (jobs, snapshots, ir) = self.workers.live_usage();
        self.peak_compiler_bytes = self.peak_compiler_bytes.max(snapshots.saturating_add(ir));
        self.coordinator.set_worker_usage(jobs, snapshots, ir);
        let mut snapshot = self.coordinator.metrics();
        snapshot.native_entries = self.native_entries;
        snapshot.native_exits = self.native_exits;
        snapshot.native_fallbacks = self.native_fallbacks;
        snapshot.native_retries = self.native_retries;
        self.osr_entries = self.osr_validation.successes.load(Ordering::Relaxed);
        self.osr_validation_failures = self.osr_validation.failures.load(Ordering::Relaxed);
        snapshot.osr_entries = self.osr_entries;
        snapshot.osr_attempts = self.osr_attempts;
        snapshot.osr_validated_successes = self.osr_validation.successes.load(Ordering::Relaxed);
        snapshot.osr_generated_retries = self.osr_generated_retries;
        snapshot.hot_call_queues = self.hot_call_queues;
        snapshot.hot_loop_queues = self.hot_loop_queues;
        snapshot.adaptive_neutral_queues = self.adaptive_neutral_queues;
        snapshot.adaptive_inputs_recorded = self.adaptive_inputs_recorded;
        snapshot.adaptive_size_factor_disabled = self.adaptive_inputs_recorded;
        snapshot.snapshot_requests = self.snapshot_requests;
        snapshot.stable_path_compile_requests = self.stable_path_compile_requests;
        snapshot.profitability_evaluations = self.profitability_evaluations;
        snapshot.profitability_approved = self.profitability_approved;
        snapshot.profitability_rejected = self.profitability_rejected;
        snapshot.benefit_recordings = self.benefit_recordings;
        snapshot.measured_benefit_ns = self.measured_benefit_ns;
        snapshot.compile_ns = self
            .compiler_measurements
            .elapsed_ns
            .load(Ordering::Relaxed);
        snapshot.install_ns = self.install_ns;
        snapshot.peak_compiler_bytes = self.peak_compiler_bytes;
        snapshot.osr_not_ready = self.osr_not_ready;
        snapshot.osr_map_misses = self.osr_map_misses;
        snapshot.osr_validation_failures = self.osr_validation_failures;
        snapshot.deopt_materializations = self
            .osr_validation
            .deopt_materializations
            .load(Ordering::Relaxed);
        let requested = self.requested.iter().copied().collect::<Vec<_>>();
        for key in requested {
            let retryable = match self.coordinator.tier_state(key, runtime::Tier::Baseline) {
                runtime::CompileState::Cold | runtime::CompileState::Retired => true,
                runtime::CompileState::Backoff { retry_after, .. } => self.clock >= retry_after,
                _ => false,
            };
            if retryable {
                self.clear_failed_request(key);
            }
        }
        *self.metrics.lock().unwrap_or_else(|p| p.into_inner()) = snapshot.clone();
    }

    fn clear_failed_request(&mut self, key: runtime::FunctionKey) {
        self.requested.remove(&key);
        self.queue_reasons.remove(&key);
        if let Some(hotness) = self.hotness.get_mut(&key) {
            hotness.clear_queued();
        }
    }

    fn fail_before_queue(&mut self, key: runtime::FunctionKey) {
        let attempts = self
            .prequeue_backoff
            .get(&key)
            .map_or(1, |(attempts, _)| attempts.saturating_add(1));
        let delay = 1_u64 << u32::from(attempts.min(16));
        self.prequeue_backoff
            .insert(key, (attempts, self.clock.saturating_add(delay)));
        self.clear_failed_request(key);
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
        if event.count == 0 {
            return 0;
        }
        let key = runtime::FunctionKey::new(event.function.id, event.function.generation);
        let observed = match event.feedback_type {
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_INT32 => {
                Some(runtime::ObservedType::Int32)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_FLOAT64 => {
                Some(runtime::ObservedType::Float64)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_BOOL => {
                Some(runtime::ObservedType::Bool)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_NULL => {
                Some(runtime::ObservedType::Null)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_UNDEFINED => {
                Some(runtime::ObservedType::Undefined)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_STRING => {
                Some(runtime::ObservedType::String)
            }
            rquickjs_core::qjs::JSJitFeedbackType_JS_JIT_FEEDBACK_OBJECT => {
                Some(runtime::ObservedType::Object)
            }
            _ => None,
        };
        if event.feedback_slot != u32::MAX {
            if let Some(observed) = observed {
                self.feedback
                    .observe_type(key, event.pc, runtime::FeedbackKind::Value, observed);
            }
        }
        if event.callee.id != 0 {
            self.feedback.observe_type(
                key,
                event.pc,
                runtime::FeedbackKind::CallTarget,
                runtime::ObservedType::Function(runtime::FunctionKey::new(
                    event.callee.id,
                    event.callee.generation,
                )),
            );
        }
        if matches!(
            self.coordinator.tier_state(key, runtime::Tier::Baseline),
            runtime::CompileState::Installed(_)
        ) {
            if !matches!(
                self.coordinator.tier_state(key, runtime::Tier::Optimizing),
                runtime::CompileState::Cold | runtime::CompileState::Backoff { .. }
            ) || self.optimizing_requested.contains(&key)
            {
                return 0;
            }
            let thresholds = runtime::HotThresholds {
                calls: self.config.call_threshold(),
                loops: self.config.loop_threshold(),
                rationale: runtime::HotReason::NeutralBase,
            };
            let hotness = self.optimizing_hotness.entry(key).or_default();
            let decision = if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_CALL
                && event.pc == 0
            {
                hotness.record_call_event_with_thresholds(event.count, thresholds)
            } else if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_LOOP
                && event.pc != 0
            {
                hotness.record_loop_event_with_thresholds(event.count, thresholds)
            } else {
                return 0;
            };
            if matches!(decision, runtime::HotDecision::Queue(_)) {
                self.optimizing_requested.insert(key);
                self.snapshot_requests = self.snapshot_requests.saturating_add(1);
                return 1;
            }
            return 0;
        }
        if matches!(
            self.coordinator.tier_state(key, runtime::Tier::Baseline),
            runtime::CompileState::Blacklisted | runtime::CompileState::Installed(_)
        ) {
            self.requested.insert(key);
            return 0;
        }
        if self
            .prequeue_backoff
            .get(&key)
            .is_some_and(|(_, retry_after)| self.clock < *retry_after)
        {
            return 0;
        }
        if self.requested.contains(&key) {
            return 0;
        }
        let hotness = self.hotness.entry(key).or_default();
        // Task 14 owns evidence-based coefficients. Production still consumes
        // the adaptive interface now, with its documented neutral inputs.
        let adaptive = runtime::AdaptiveInputs::default().thresholds();
        let thresholds = runtime::HotThresholds {
            calls: self.config.call_threshold(),
            loops: self.config.loop_threshold(),
            rationale: adaptive.rationale,
        };
        let decision = if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_CALL
            && event.pc == 0
        {
            hotness.record_call_event_with_thresholds(event.count, thresholds)
        } else if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_LOOP && event.pc != 0 {
            hotness.record_loop_event_with_thresholds(event.count, thresholds)
        } else {
            return 0;
        };
        if let runtime::HotDecision::Queue(reason) = decision {
            self.requested.insert(key);
            self.queue_reasons.insert(key, reason);
            self.snapshot_requests = self.snapshot_requests.saturating_add(1);
            1
        } else {
            0
        }
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
        let raw_key = unsafe {
            runtime::FunctionKey::new(raw.as_ref().function.id, raw.as_ref().function.generation)
        };
        let _free = FreeSnapshot(snapshot);
        let copied = unsafe { bytecode::CompileSnapshot::copy_borrowed_raw(raw.as_ref()) };
        let Ok(snapshot) = copied else {
            self.fail_before_queue(raw_key);
            return;
        };
        let key = runtime::FunctionKey::new(snapshot.function_id(), snapshot.generation());
        if snapshot.owned_bytes() > self.config.max_snapshot_bytes() {
            self.coordinator.record_resource_limit_rejection();
            self.fail_before_queue(key);
            self.maintenance();
            return;
        }
        let Ok(verified) = snapshot.verify(bytecode::VerifyLimits::default()) else {
            self.fail_before_queue(key);
            return;
        };
        let adaptive = runtime::AdaptiveInputs {
            bytecode_bytes: verified
                .snapshot()
                .bytecode()
                .len()
                .try_into()
                .unwrap_or(u32::MAX),
            helper_ops: 0,
            instruction_count: verified.instructions().len().try_into().unwrap_or(u32::MAX),
            measured_work: None,
        };
        let profile = self.execution_profiles.entry(key).or_default();
        profile.bytecodes = verified.instructions().len().try_into().unwrap_or(u64::MAX);
        profile.helper_calls = verified
            .instructions()
            .iter()
            .filter(|instruction| {
                matches!(
                    bytecode::tier1_policy(instruction.opcode().id()),
                    Some(bytecode::Tier1Policy::Helper(_))
                )
            })
            .count()
            .try_into()
            .unwrap_or(u64::MAX);
        self.adaptive_inputs_recorded = self.adaptive_inputs_recorded.saturating_add(1);
        debug_assert_eq!(
            adaptive.thresholds().rationale,
            runtime::HotReason::NeutralBase
        );
        let requested_tier = if self.optimizing_requested.contains(&key) {
            runtime::Tier::Optimizing
        } else {
            runtime::Tier::Baseline
        };
        let tier2_snapshot = (requested_tier == runtime::Tier::Baseline).then(|| verified.clone());
        let queued = self.coordinator.queue(key, requested_tier, verified);
        match queued {
            Ok(()) => {
                match self.queue_reasons.get(&key).copied() {
                    Some(runtime::HotReason::CallThreshold) => {
                        self.hot_call_queues = self.hot_call_queues.saturating_add(1);
                    }
                    Some(runtime::HotReason::LoopThreshold) => {
                        self.hot_loop_queues = self.hot_loop_queues.saturating_add(1);
                    }
                    _ => {}
                }
                self.adaptive_neutral_queues = self.adaptive_neutral_queues.saturating_add(1);
                self.prequeue_backoff.remove(&key);
                if let Some(snapshot) = tier2_snapshot {
                    self.optimizing_snapshots.insert(key, snapshot);
                }
            }
            Err(runtime::QueueError::Blacklisted | runtime::QueueError::NotReady) => {
                // An active/installed/blacklisted coordinator state owns the
                // request bit; retaining it prevents duplicate snapshots.
                self.requested.insert(key);
            }
            Err(runtime::QueueError::Retired | runtime::QueueError::Shutdown) => {
                self.clear_failed_request(key);
            }
            Err(_) => self.fail_before_queue(key),
        }
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
        let key = runtime::FunctionKey::new(id, generation);
        let acquired_tier = if matches!(
            self.coordinator.tier_state(key, runtime::Tier::Optimizing),
            runtime::CompileState::Installed(_)
        ) {
            runtime::Tier::Optimizing
        } else {
            runtime::Tier::Baseline
        };
        let Some(pin) = self.coordinator.pin(key, acquired_tier) else {
            if pc != 0 {
                self.osr_not_ready = self.osr_not_ready.saturating_add(1);
            }
            return empty;
        };
        let Some(published) = pin.artifact().published() else {
            if pc != 0 {
                self.osr_not_ready = self.osr_not_ready.saturating_add(1);
            }
            return empty;
        };
        let (published, osr_map) = if pc == 0 {
            (published, None)
        } else {
            let Some((map, osr)) = published.osr_entry(pc) else {
                self.osr_map_misses = self.osr_map_misses.saturating_add(1);
                return empty;
            };
            if map.key().function() != runtime::FunctionKey::new(id, generation) {
                self.osr_map_misses = self.osr_map_misses.saturating_add(1);
                return empty;
            }
            (osr, Some(map.clone()))
        };
        #[cfg(feature = "test-support")]
        {
            *self
                .test_last_acquired_key
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pin.artifact().key());
        }
        let entry = published.as_ptr();
        let stack_map_count = published.required_stack_map_count();
        let artifact_key = pin.artifact().key();
        let deopt_sites: Box<[(ir::OptimizedFrameShape, ir::DeoptMap)]> =
            pin.artifact().optimized_metadata().map_or_else(
                || Vec::new().into_boxed_slice(),
                |metadata| metadata.deopt_sites().to_vec().into_boxed_slice(),
            );
        let pin = Box::into_raw(Box::new(ProductionEntryPin {
            execution: pin,
            native: entry,
            runtime_id: artifact_key.runtime_id,
            key: runtime::FunctionKey::new(id, generation),
            pc,
            stack_map_count,
            osr: osr_map,
            deopt_sites,
            validation: Arc::clone(&self.osr_validation),
            #[cfg(feature = "test-support")]
            stress_gc: self.config.stress_gc(),
        }))
        .cast();
        empty.entry = Some(production_entry_trampoline);
        empty.pin = pin;
        empty.stack_map_count = stack_map_count;
        empty.helper_abi_version = rquickjs_core::qjs::QJSJIT_HELPER_ABI_VERSION;
        self.pending_entry_tiers
            .entry(key)
            .or_default()
            .push_back(acquired_tier);
        empty
    }

    fn release_entry(&mut self, entry: rquickjs_core::qjs::JSJitEntryHandle) {
        if !entry.pin.is_null() {
            unsafe { drop(Box::from_raw(entry.pin.cast::<ProductionEntryPin>())) };
        }
    }

    fn native_enter(&mut self, _id: u64, _generation: u64, pc: u32) {
        self.native_entries = self.native_entries.saturating_add(1);
        let key = runtime::FunctionKey::new(_id, _generation);
        let tier = self
            .pending_entry_tiers
            .get_mut(&key)
            .and_then(std::collections::VecDeque::pop_front)
            .unwrap_or(runtime::Tier::Baseline);
        if tier == runtime::Tier::Optimizing {
            self.coordinator.record_tier2_entry();
        }
        if pc != 0 {
            self.osr_attempts = self.osr_attempts.saturating_add(1);
        }
        self.execution_starts
            .entry(key)
            .or_default()
            .push((std::time::Instant::now(), tier));
    }

    fn native_exit(&mut self, id: u64, generation: u64, pc: u32, exit_kind: u32) {
        self.native_exits = self.native_exits.saturating_add(1);
        let key = runtime::FunctionKey::new(id, generation);
        if let Some((start, tier)) = self.execution_starts.get_mut(&key).and_then(Vec::pop) {
            let elapsed = start.elapsed().as_nanos().try_into().unwrap_or(u64::MAX);
            let optimized = tier == runtime::Tier::Optimizing;
            let profile = self.execution_profiles.entry(key).or_default();
            if optimized {
                profile.optimized_executions = profile.optimized_executions.saturating_add(1);
                profile.optimized_ns = profile.optimized_ns.saturating_add(elapsed);
                if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DONE {
                    if let Some(baseline_average) =
                        profile.baseline_ns.checked_div(profile.baseline_executions)
                    {
                        let saved = baseline_average.saturating_sub(elapsed);
                        if saved != 0
                            && self.coordinator.record_benefit(
                                key,
                                runtime::Tier::Optimizing,
                                saved,
                            )
                        {
                            self.benefit_recordings = self.benefit_recordings.saturating_add(1);
                            self.measured_benefit_ns =
                                self.measured_benefit_ns.saturating_add(saved);
                        }
                    }
                }
            } else {
                profile.baseline_executions = profile.baseline_executions.saturating_add(1);
                profile.baseline_ns = profile.baseline_ns.saturating_add(elapsed);
            }
        }
        self.coordinator
            .record_side_path_entries(self.osr_validation.take_side_path_entries());
        if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER {
            self.native_retries = self.native_retries.saturating_add(1);
            let validation_retry = self
                .osr_validation
                .take_validation_retry(id, generation, pc);
            if is_generated_osr_retry(pc, exit_kind, validation_retry) {
                self.osr_generated_retries = self.osr_generated_retries.saturating_add(1);
            }
        } else if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT {
            self.native_fallbacks = self.native_fallbacks.saturating_add(1);
            let key = runtime::FunctionKey::new(id, generation);
            if let Some((guard, observed)) = self.osr_validation.take_deopt_guard(id, generation) {
                if self
                    .coordinator
                    .record_optimized_side_exit_profile(key, guard, observed)
                    == runtime::SideExitAction::StablePathThreshold
                {
                    if let Some(snapshot) = self.tier2_sources.get(&key).cloned() {
                        let guard_pc = self
                            .coordinator
                            .pin(key, runtime::Tier::Optimizing)
                            .and_then(|pin| {
                                pin.artifact().optimized_metadata().and_then(|metadata| {
                                    metadata.deopt_sites().iter().find_map(|(_, map)| {
                                        (map.guard() == guard).then_some(map.resume_pc())
                                    })
                                })
                            });
                        if let (Some(guard_pc), Some(observed)) = (guard_pc, observed) {
                            self.feedback.observe_type(
                                key,
                                guard_pc,
                                runtime::FeedbackKind::Exit,
                                observed,
                            );
                        }
                        let feedback = self.feedback.snapshot(self.clock.max(1));
                        let profile = guard_pc.zip(observed).map(|(guard_pc, observed)| {
                            runtime::SidePathProfile::new(
                                key,
                                runtime::GuardId::new(guard),
                                guard_pc,
                                observed,
                                feedback.epoch(),
                            )
                        });
                        if profile.is_some_and(|profile| {
                            self.coordinator
                                .queue_side_path(key, snapshot, feedback, profile)
                                .is_ok()
                        }) {
                            self.stable_path_compile_requests =
                                self.stable_path_compile_requests.saturating_add(1);
                        }
                    }
                }
            } else {
                self.coordinator.record_deopt(true);
            }
        }
        self.maintenance();
    }

    fn function_retire(&mut self, id: u64, generation: u64) {
        let key = runtime::FunctionKey::new(id, generation);
        self.requested.remove(&key);
        self.queue_reasons.remove(&key);
        self.prequeue_backoff.remove(&key);
        self.hotness.remove(&key);
        self.optimizing_requested.remove(&key);
        self.optimizing_hotness.remove(&key);
        self.optimizing_snapshots.remove(&key);
        self.tier2_sources.remove(&key);
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
        Ok(JitRuntime {
            jit: Some(jit),
            runtime: Some(runtime),
        })
    }
}

/// An owning QuickJS runtime paired with its JIT guard.
pub struct JitRuntime {
    jit: Option<Jit>,
    runtime: Option<Runtime>,
}

impl JitRuntime {
    /// Starts building an owning JIT runtime.
    pub fn builder() -> JitRuntimeBuilder {
        JitRuntimeBuilder::default()
    }

    /// Returns the disabled JIT guard.
    pub const fn jit(&self) -> &Jit {
        match &self.jit {
            Some(jit) => jit,
            None => panic!("JIT guard is present until drop"),
        }
    }

    /// Returns runtime metrics.
    pub fn metrics(&self) -> JitMetrics {
        self.jit().metrics()
    }
}

impl Deref for JitRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("QuickJS runtime is present until drop")
    }
}

fn drop_jit_before_runtime<J, R>(jit: &mut Option<J>, runtime: &mut Option<R>) {
    drop(jit.take());
    drop(runtime.take());
}

impl Drop for JitRuntime {
    fn drop(&mut self) {
        drop_jit_before_runtime(&mut self.jit, &mut self.runtime);
    }
}

#[cfg(test)]
mod jit_runtime_drop_tests {
    use super::drop_jit_before_runtime;
    use std::sync::{Arc, Mutex};

    struct DropProbe {
        label: &'static str,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.events.lock().unwrap().push(self.label);
        }
    }

    #[test]
    fn owning_runtime_drops_jit_guard_before_quickjs_runtime() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut jit = Some(DropProbe {
            label: "jit",
            events: Arc::clone(&events),
        });
        let mut runtime = Some(DropProbe {
            label: "runtime",
            events: Arc::clone(&events),
        });

        drop_jit_before_runtime(&mut jit, &mut runtime);

        assert_eq!(*events.lock().unwrap(), ["jit", "runtime"]);
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
