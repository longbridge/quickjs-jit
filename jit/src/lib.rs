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
    hotness: std::collections::HashMap<runtime::FunctionKey, runtime::HotnessState>,
    metrics: Arc<Mutex<JitMetrics>>,
    native_entries: u64,
    native_exits: u64,
    native_fallbacks: u64,
    native_retries: u64,
    osr_entries: u64,
    osr_not_ready: u64,
    osr_map_misses: u64,
    osr_validation_failures: u64,
    #[cfg(feature = "test-support")]
    test_last_acquired_key: Arc<Mutex<Option<code_cache::ArtifactKey>>>,
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
    let frame = unsafe { &*frame };
    if frame.entry.pin.is_null() {
        return retry_exit();
    }
    let pin = unsafe { &*frame.entry.pin.cast::<ProductionEntryPin>() };
    let _keep_execution_pinned = &pin.execution;
    if !validate_production_frame(
        frame,
        pin.runtime_id,
        pin.key,
        pin.pc,
        pin.stack_map_count,
        pin.osr.as_ref(),
    ) {
        return retry_exit();
    }
    type NativeEntry = unsafe extern "C" fn(*mut qjs::JSJitExecFrame) -> qjs::JSJitExit;
    let native = unsafe { core::mem::transmute::<*const u8, NativeEntry>(pin.native) };
    unsafe { native(frame as *const _ as *mut _) }
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
            hotness: std::collections::HashMap::new(),
            metrics,
            native_entries: 0,
            native_exits: 0,
            native_fallbacks: 0,
            native_retries: 0,
            osr_entries: 0,
            osr_not_ready: 0,
            osr_map_misses: 0,
            osr_validation_failures: 0,
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
        snapshot.osr_entries = self.osr_entries;
        snapshot.osr_not_ready = self.osr_not_ready;
        snapshot.osr_map_misses = self.osr_map_misses;
        snapshot.osr_validation_failures = self.osr_validation_failures;
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
        if event.count == 0 {
            return 0;
        }
        let key = runtime::FunctionKey::new(event.function.id, event.function.generation);
        if self.requested.contains(&key) {
            return 0;
        }
        let hotness = self.hotness.entry(key).or_default();
        let decision = if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_CALL
            && event.pc == 0
        {
            hotness.record_call_event(event.count)
        } else if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_LOOP && event.pc != 0 {
            hotness.record_loop_event(event.count)
        } else {
            return 0;
        };
        if matches!(decision, runtime::HotDecision::Queue(_)) {
            self.requested.insert(key);
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
        let Some(pin) = self.coordinator.pin(
            runtime::FunctionKey::new(id, generation),
            runtime::Tier::Baseline,
        ) else {
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
        let pin = Box::into_raw(Box::new(ProductionEntryPin {
            execution: pin,
            native: entry,
            runtime_id: artifact_key.runtime_id,
            key: runtime::FunctionKey::new(id, generation),
            pc,
            stack_map_count,
            osr: osr_map,
        }))
        .cast();
        empty.entry = Some(production_entry_trampoline);
        empty.pin = pin;
        empty.stack_map_count = stack_map_count;
        empty.helper_abi_version = rquickjs_core::qjs::QJSJIT_HELPER_ABI_VERSION;
        empty
    }

    fn release_entry(&mut self, entry: rquickjs_core::qjs::JSJitEntryHandle) {
        if !entry.pin.is_null() {
            unsafe { drop(Box::from_raw(entry.pin.cast::<ProductionEntryPin>())) };
        }
    }

    fn native_enter(&mut self, _id: u64, _generation: u64, pc: u32) {
        self.native_entries = self.native_entries.saturating_add(1);
        if pc != 0 {
            self.osr_entries = self.osr_entries.saturating_add(1);
        }
    }

    fn native_exit(&mut self, _id: u64, _generation: u64, pc: u32, exit_kind: u32) {
        self.native_exits = self.native_exits.saturating_add(1);
        if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER {
            self.native_retries = self.native_retries.saturating_add(1);
            if pc != 0 {
                self.osr_validation_failures = self.osr_validation_failures.saturating_add(1);
            }
        } else if exit_kind == rquickjs_core::qjs::JSJitExitKind_JS_JIT_EXIT_DEOPT {
            self.native_fallbacks = self.native_fallbacks.saturating_add(1);
        }
        self.maintenance();
    }

    fn function_retire(&mut self, id: u64, generation: u64) {
        let key = runtime::FunctionKey::new(id, generation);
        self.requested.remove(&key);
        self.hotness.remove(&key);
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
