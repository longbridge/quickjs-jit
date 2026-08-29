use std::{
    collections::HashMap,
    mem, ptr,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use rquickjs_core::runtime::{JitBackend, JitBackendAttachError, RuntimeJitGuard};
use rquickjs_core::{context::EvalOptions, Context, Runtime, Value};

use crate::abi::{AbiInfo, AbiMismatch, AbiStructure};
use crate::bytecode::{
    opcode, CompileSnapshot, DeoptPoint, OsrPoint, RuntimeConstants, SnapshotStatus,
    VerifiedFunction, VerifierMetadata, VerifyLimits,
};
use crate::code_cache::CompiledArtifact;
use crate::compiler::{mock::FakeCompiler, mock::FakeCompilerControl, Compiler};
use crate::runtime::{
    CompileCompletion, CompileFailure, CompileRequest, CompileState, Coordinator, FunctionKey, Tier,
};
use crate::{Jit, JitConfig, JitDiagnosticKind, JitError, JitMetrics};

pub use crate::bytecode::decode_raw;

fn coordinator_snapshot() -> VerifiedFunction {
    CompileSnapshot::from_untrusted_bytecode(vec![opcode::RETURN_UNDEF], 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .expect("coordinator fixture verifies")
}

pub struct Harness {
    coordinator: Mutex<Coordinator>,
    requests: Mutex<HashMap<FunctionKey, CompileRequest>>,
    compiler: FakeCompiler,
    control: FakeCompilerControl,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        Self::with_max_attempts(4)
    }

    pub fn with_max_attempts(max_attempts: u8) -> Self {
        let (compiler, control) = FakeCompiler::new(8);
        Self {
            coordinator: Mutex::new(Coordinator::with_limits(8, 8, max_attempts, 3)),
            requests: Mutex::new(HashMap::new()),
            compiler,
            control,
        }
    }

    pub fn queue(&self, key: FunctionKey) {
        let request = {
            let mut coordinator = self
                .coordinator
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            coordinator
                .queue(key, Tier::Baseline, coordinator_snapshot())
                .expect("harness queue accepted");
            coordinator.begin_next().expect("harness request begins")
        };
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, request);
    }

    pub fn retire(&self, key: FunctionKey) {
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retire(key);
    }

    pub fn complete(&self, key: FunctionKey, artifact: CompiledArtifact) {
        let request = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key)
            .expect("harness request exists");
        let requested_tier = request.tier();
        let artifact_key = request.artifact_key();
        let attempt_id = request.attempt_id();
        self.control.complete(artifact);
        let result = self.compiler.compile(request);
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .complete(CompileCompletion {
                key,
                requested_tier,
                artifact_key,
                attempt_id,
                result,
            });
    }

    pub fn fail(&self, key: FunctionKey, failure: CompileFailure) {
        let request = {
            let mut coordinator = self
                .coordinator
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let CompileState::Backoff { retry_after, .. } = coordinator.state(key) {
                coordinator.advance_clock(retry_after);
            }
            coordinator
                .queue(key, Tier::Baseline, coordinator_snapshot())
                .expect("harness failure queued");
            coordinator.begin_next().expect("harness failure begins")
        };
        let requested_tier = request.tier();
        let artifact_key = request.artifact_key();
        let attempt_id = request.attempt_id();
        self.control.fail(failure);
        let result = self.compiler.compile(request);
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .complete(CompileCompletion {
                key,
                requested_tier,
                artifact_key,
                attempt_id,
                result,
            });
    }

    pub fn state(&self, key: FunctionKey) -> CompileState {
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state(key)
    }

    pub fn metrics(&self) -> JitMetrics {
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .metrics()
            .clone()
    }
}

pub struct SnapshotFixture {
    runtime: Runtime,
    _context: Context,
    snapshot: CompileSnapshot,
    _runtime_constants: RuntimeConstants,
}

impl SnapshotFixture {
    pub fn compile(source: &str) -> Self {
        let runtime = Runtime::new().expect("snapshot runtime");
        let context = Context::full(&runtime).expect("snapshot context");
        let snapshot = context.with(|ctx| {
            let mut options = EvalOptions::default();
            options.global = true;
            options.strict = false;
            let function: Value<'_> = ctx
                .eval_with_options(source, options)
                .expect("compile snapshot fixture");
            unsafe {
                CompileSnapshot::capture_with_runtime_constants(
                    &runtime,
                    ctx.as_raw().as_ptr(),
                    function.as_raw(),
                )
            }
            .expect("snapshot supported function")
        });
        let (snapshot, runtime_constants) = snapshot;
        Self {
            runtime,
            _context: context,
            snapshot,
            _runtime_constants: runtime_constants,
        }
    }

    pub fn snapshot(&self) -> CompileSnapshot {
        self.snapshot.clone()
    }
}

impl Drop for SnapshotFixture {
    fn drop(&mut self) {
        self.runtime.run_gc();
    }
}

pub fn snapshot_status(source: &str) -> SnapshotStatus {
    let runtime = Runtime::new().expect("snapshot runtime");
    let context = Context::full(&runtime).expect("snapshot context");
    context.with(|ctx| {
        let mut options = EvalOptions::default();
        options.global = true;
        options.strict = false;
        let function: Value<'_> = ctx
            .eval_with_options(source, options)
            .expect("compile snapshot status fixture");
        match unsafe { CompileSnapshot::capture_raw(ctx.as_raw().as_ptr(), function.as_raw()) } {
            Ok(_) => SnapshotStatus::Ok,
            Err(status) => status,
        }
    })
}

pub fn snapshot_from_parts(
    bytecode: Vec<u8>,
    arg_count: u16,
    local_count: u16,
    closure_count: u16,
    constant_count: u32,
) -> CompileSnapshot {
    CompileSnapshot::from_untrusted_bytecode(
        bytecode,
        arg_count,
        local_count,
        closure_count,
        constant_count,
    )
}

/// Builds the smallest worker-safe verified function used by compiler tests.
pub fn verified_bytecode(bytecode: Vec<u8>, arg_count: u16, local_count: u16) -> VerifiedFunction {
    CompileSnapshot::from_untrusted_bytecode(bytecode, arg_count, local_count, 0, 0)
        .verify(VerifyLimits::default())
        .expect("synthetic bytecode verifies")
}

/// Stable Rust representation of QuickJS's 16-byte non-NaN-boxed value ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JSValueRepr {
    pub payload: u64,
    pub tag: i64,
}

impl JSValueRepr {
    pub const fn new(payload: u64, tag: i64) -> Self {
        Self { payload, tag }
    }

    pub const fn undefined() -> Self {
        Self::new(0, rquickjs_core::qjs::JS_TAG_UNDEFINED as i64)
    }

    pub const fn int32(value: i32) -> Self {
        Self::new(value as i64 as u64, rquickjs_core::qjs::JS_TAG_INT as i64)
    }

    pub const fn float64(value: f64) -> Self {
        Self::new(value.to_bits(), rquickjs_core::qjs::JS_TAG_FLOAT64 as i64)
    }

    pub fn as_f64(self) -> Option<f64> {
        (self.tag == rquickjs_core::qjs::JS_TAG_FLOAT64 as i64)
            .then(|| f64::from_bits(self.payload))
    }

    fn into_raw(self) -> rquickjs_core::qjs::JSValue {
        // The ABI validation tests independently prove both layouts are 16
        // bytes with the tag at byte 8.
        unsafe { mem::transmute(self) }
    }

    pub fn from_raw(value: rquickjs_core::qjs::JSValue) -> Self {
        unsafe { mem::transmute(value) }
    }
}

const _: () =
    assert!(mem::size_of::<JSValueRepr>() == mem::size_of::<rquickjs_core::qjs::JSValue>());
const _: () =
    assert!(mem::offset_of!(JSValueRepr, tag) == mem::offset_of!(rquickjs_core::qjs::JSValue, tag));

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AlignedStackSlot(JSValueRepr);

const _: () = assert!(mem::size_of::<AlignedStackSlot>() == 16);
const _: () = assert!(mem::align_of::<AlignedStackSlot>() == 16);

#[derive(Debug)]
struct PollState {
    count: AtomicUsize,
    interrupt_at: AtomicUsize,
    capture_backtrace: AtomicBool,
    backtrace: Mutex<Vec<usize>>,
}

unsafe extern "C" fn synthetic_interrupt_poll(
    frame: *mut rquickjs_core::qjs::JSJitExecFrame,
) -> i32 {
    let state = unsafe { &*((*frame).rt.cast::<PollState>()) };
    let count = state.count.fetch_add(1, Ordering::AcqRel) + 1;
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if state.capture_backtrace.swap(false, Ordering::AcqRel) {
        let mut frames = [ptr::null_mut(); 64];
        let frame_count = unsafe { libc::backtrace(frames.as_mut_ptr(), frames.len() as i32) };
        let frame_count = usize::try_from(frame_count.max(0)).unwrap_or(0);
        let mut backtrace = state
            .backtrace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        backtrace.clear();
        backtrace.extend(
            frames[..frame_count]
                .iter()
                .map(|address| *address as usize),
        );
    }
    usize::from(count == state.interrupt_at.load(Ordering::Acquire)) as i32
}

static SYNTHETIC_RUNTIME_API: rquickjs_core::qjs::JSJitRuntimeAPI =
    rquickjs_core::qjs::JSJitRuntimeAPI {
        struct_size: mem::size_of::<rquickjs_core::qjs::JSJitRuntimeAPI>() as u32,
        major: rquickjs_core::qjs::QJSJIT_RUNTIME_API_MAJOR as u16,
        minor: rquickjs_core::qjs::QJSJIT_RUNTIME_API_MINOR as u16,
        interrupt_poll: Some(synthetic_interrupt_poll),
    };

/// Result observed after invoking a generated aggregate-return entry point.
#[derive(Clone, Copy, Debug)]
pub struct SyntheticOutcome {
    pub exit: rquickjs_core::qjs::JSJitExit,
    pub result: JSValueRepr,
}

/// Deep copy of every byte a generated entry can mutate through a synthetic
/// execution frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticFrameSnapshot {
    frame: Vec<u8>,
    arguments: Vec<JSValueRepr>,
    locals: Vec<JSValueRepr>,
    stack: Vec<JSValueRepr>,
    bytecode: Vec<u8>,
    poll_count: usize,
    poll_interrupt_at: usize,
    poll_capture_backtrace: bool,
    poll_backtrace: Vec<usize>,
}

/// Owns all buffers referenced by one synthetic `JSJitExecFrame`.
pub struct SyntheticFrame {
    frame: rquickjs_core::qjs::JSJitExecFrame,
    arguments: Vec<JSValueRepr>,
    locals: Vec<JSValueRepr>,
    stack: Vec<AlignedStackSlot>,
    bytecode: Vec<u8>,
    poll: Box<PollState>,
}

impl SyntheticFrame {
    pub fn new(arguments: &[JSValueRepr], local_count: usize, stack_size: usize) -> Self {
        let mut arguments = arguments.to_vec();
        let mut locals = vec![JSValueRepr::undefined(); local_count];
        // A real QuickJS value stack has allocated, suitably aligned backing
        // storage even when the live depth is zero.  Rust's empty `Vec`
        // sentinel is only aligned to `JSValueRepr`'s declared alignment and
        // is therefore not a valid synthetic `JSValue *` for the JIT's
        // stricter 16-byte slot-range contract.
        let mut stack = vec![AlignedStackSlot(JSValueRepr::undefined()); stack_size.max(1)];
        let bytecode = vec![0_u8];
        let poll = Box::new(PollState {
            count: AtomicUsize::new(0),
            interrupt_at: AtomicUsize::new(usize::MAX),
            capture_backtrace: AtomicBool::new(false),
            backtrace: Mutex::new(Vec::new()),
        });
        let frame = rquickjs_core::qjs::JSJitExecFrame {
            struct_size: mem::size_of::<rquickjs_core::qjs::JSJitExecFrame>() as u32,
            flags: 0,
            rt: (&*poll as *const PollState).cast_mut().cast(),
            ctx: ptr::null_mut(),
            function_id: 0,
            generation: 0,
            arg_buf: arguments.as_mut_ptr().cast(),
            var_buf: locals.as_mut_ptr().cast(),
            stack_base: stack.as_mut_ptr().cast(),
            stack_top: stack.as_mut_ptr().cast(),
            bytecode_start: bytecode.as_ptr(),
            pc: bytecode.as_ptr(),
            result: JSValueRepr::undefined().into_raw(),
            entry: rquickjs_core::qjs::JSJitEntryHandle {
                struct_size: mem::size_of::<rquickjs_core::qjs::JSJitEntryHandle>() as u32,
                reserved: 0,
                entry: None,
                pin: ptr::null_mut(),
            },
            runtime_api: &SYNTHETIC_RUNTIME_API,
        };
        Self {
            frame,
            arguments,
            locals,
            stack,
            bytecode,
            poll,
        }
    }

    pub fn set_bytecode(&mut self, bytecode: &[u8]) {
        self.bytecode.clear();
        self.bytecode.extend_from_slice(bytecode);
        self.frame.bytecode_start = self.bytecode.as_ptr();
        self.frame.pc = self.bytecode.as_ptr();
    }

    pub fn bytecode_start(&self) -> *const u8 {
        self.frame.bytecode_start
    }

    pub fn interrupt_on_poll(&mut self, poll: usize) {
        self.poll.interrupt_at.store(poll, Ordering::Release);
    }

    pub fn poll_count(&self) -> usize {
        self.poll.count.load(Ordering::Acquire)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub fn capture_backtrace_on_next_poll(&mut self) {
        self.poll.capture_backtrace.store(true, Ordering::Release);
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub fn captured_backtrace(&self) -> Vec<usize> {
        self.poll
            .backtrace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn snapshot(&self) -> SyntheticFrameSnapshot {
        let frame = unsafe {
            std::slice::from_raw_parts(
                ptr::addr_of!(self.frame).cast::<u8>(),
                mem::size_of::<rquickjs_core::qjs::JSJitExecFrame>(),
            )
        }
        .to_vec();
        SyntheticFrameSnapshot {
            frame,
            arguments: self.arguments.clone(),
            locals: self.locals.clone(),
            stack: self.stack.iter().map(|slot| slot.0).collect(),
            bytecode: self.bytecode.clone(),
            poll_count: self.poll.count.load(Ordering::Acquire),
            poll_interrupt_at: self.poll.interrupt_at.load(Ordering::Acquire),
            poll_capture_backtrace: self.poll.capture_backtrace.load(Ordering::Acquire),
            poll_backtrace: self
                .poll
                .backtrace
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        }
    }

    pub fn set_local(&mut self, index: usize, value: JSValueRepr) {
        self.locals[index] = value;
    }

    pub fn set_stack(&mut self, values: &[JSValueRepr]) {
        assert!(values.len() <= self.stack.len());
        for (slot, value) in self.stack.iter_mut().zip(values) {
            slot.0 = *value;
        }
        self.frame.stack_top = unsafe { self.frame.stack_base.add(values.len()) };
    }

    pub fn stack_storage_address(&self) -> usize {
        self.stack.as_ptr() as usize
    }

    pub fn set_stack_bounds_raw(&mut self, base: usize, top: usize) {
        self.frame.stack_base = base as *mut rquickjs_core::qjs::JSValue;
        self.frame.stack_top = top as *mut rquickjs_core::qjs::JSValue;
    }

    /// Invokes an entry whose Cranelift signature has an sret pointer followed
    /// by the execution-frame pointer and no ordinary return values.
    #[cfg(feature = "compiler")]
    pub unsafe fn call(
        &mut self,
        executable: &crate::compiler::baseline::PublishedBaselineCode,
    ) -> SyntheticOutcome {
        type Entry = unsafe extern "C" fn(
            *mut rquickjs_core::qjs::JSJitExecFrame,
        ) -> rquickjs_core::qjs::JSJitExit;
        let entry: Entry = unsafe { mem::transmute(executable.as_ptr()) };
        let exit = unsafe { entry(ptr::addr_of_mut!(self.frame)) };
        SyntheticOutcome {
            exit,
            result: JSValueRepr::from_raw(self.frame.result),
        }
    }
}

impl Drop for SyntheticFrame {
    fn drop(&mut self) {
        // Read the owned buffers so dead-code analysis cannot obscure that
        // their allocations intentionally pin every pointer in `frame`.
        let _ = (&self.arguments, &self.locals, &self.stack, &self.bytecode);
    }
}

pub fn verifier_metadata(
    osr_points: Vec<OsrPoint>,
    deopt_points: Vec<DeoptPoint>,
) -> VerifierMetadata {
    VerifierMetadata::new(osr_points, deopt_points)
}

#[derive(Clone)]
pub struct LifecycleRecorder {
    events: Arc<Mutex<Vec<&'static str>>>,
}

pub struct LifecycleRuntime {
    _guard: RuntimeJitGuard,
    runtime: Runtime,
}

impl LifecycleRuntime {
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
}

struct RecordingBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
}

unsafe impl JitBackend for RecordingBackend {
    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push("detach");
    }
}

impl Drop for RecordingBackend {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("backend_drop");
    }
}

pub fn record_lifecycle() -> LifecycleRecorder {
    LifecycleRecorder {
        events: Arc::new(Mutex::new(Vec::new())),
    }
}

impl LifecycleRecorder {
    pub fn runtime(&self) -> LifecycleRuntime {
        let runtime = Runtime::new().expect("test runtime");
        let drop_events = Arc::clone(&self.events);
        rquickjs_core::runtime::test_support::set_runtime_drop_probe(&runtime, move || {
            drop_events.lock().unwrap().push("runtime_drop");
        });
        let guard = RuntimeJitGuard::attach(
            &runtime,
            RecordingBackend {
                events: Arc::clone(&self.events),
            },
        )
        .expect("attach test backend");
        self.events.lock().unwrap().push("attach");
        LifecycleRuntime {
            _guard: guard,
            runtime,
        }
    }

    pub fn snapshot(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }

    pub fn take(&self) -> Vec<&'static str> {
        let mut events = self.events.lock().unwrap();
        core::mem::take(&mut *events)
    }
}

pub const fn fresh_bindgen_bindings() -> Option<&'static str> {
    rquickjs_core::runtime::test_support::fresh_bindgen_bindings()
}

struct DetachLabelBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    label: &'static str,
}

unsafe impl JitBackend for DetachLabelBackend {
    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push(self.label);
    }
}

pub fn duplicate_attachment_is_rejected() -> bool {
    let runtime = Runtime::new().expect("test runtime");
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = RuntimeJitGuard::attach(
        &runtime,
        DetachLabelBackend {
            events: Arc::clone(&events),
            label: "first_detach",
        },
    )
    .expect("first attachment");
    let second = RuntimeJitGuard::attach(
        &runtime,
        DetachLabelBackend {
            events: Arc::clone(&events),
            label: "second_detach",
        },
    );
    let rejected = matches!(second, Err(JitBackendAttachError::AlreadyAttached));
    drop(first);
    rejected && *events.lock().unwrap() == ["first_detach"]
}

#[derive(Clone, Copy, Debug)]
pub enum AbiMismatchFixture {
    SourceRevision,
    OpcodeFingerprint,
    ValueLayout,
    FeatureFlags,
    PointerWidth,
    Endianness,
    AbiInfoLayout,
    FunctionIdLayout,
    HotEventLayout,
    FunctionSnapshotLayout,
    EntryHandleLayout,
    ExecFrameLayout,
    ExitLayout,
    RuntimeApiLayout,
    BackendVTableLayout,
}

impl AbiMismatchFixture {
    pub const ALL: [Self; 15] = [
        Self::SourceRevision,
        Self::OpcodeFingerprint,
        Self::ValueLayout,
        Self::FeatureFlags,
        Self::PointerWidth,
        Self::Endianness,
        Self::AbiInfoLayout,
        Self::FunctionIdLayout,
        Self::HotEventLayout,
        Self::FunctionSnapshotLayout,
        Self::EntryHandleLayout,
        Self::ExecFrameLayout,
        Self::ExitLayout,
        Self::RuntimeApiLayout,
        Self::BackendVTableLayout,
    ];

    const fn mismatch(self) -> AbiMismatch {
        match self {
            Self::SourceRevision => AbiMismatch::SourceRevision,
            Self::OpcodeFingerprint => AbiMismatch::OpcodeFingerprint,
            Self::ValueLayout => AbiMismatch::ValueLayout,
            Self::FeatureFlags => AbiMismatch::FeatureFlags,
            Self::PointerWidth => AbiMismatch::PointerWidth,
            Self::Endianness => AbiMismatch::Endianness,
            Self::AbiInfoLayout => AbiMismatch::StructureLayout(AbiStructure::AbiInfo),
            Self::FunctionIdLayout => AbiMismatch::StructureLayout(AbiStructure::FunctionId),
            Self::HotEventLayout => AbiMismatch::StructureLayout(AbiStructure::HotEvent),
            Self::FunctionSnapshotLayout => {
                AbiMismatch::StructureLayout(AbiStructure::FunctionSnapshot)
            }
            Self::EntryHandleLayout => AbiMismatch::StructureLayout(AbiStructure::EntryHandle),
            Self::ExecFrameLayout => AbiMismatch::StructureLayout(AbiStructure::ExecFrame),
            Self::ExitLayout => AbiMismatch::StructureLayout(AbiStructure::Exit),
            Self::RuntimeApiLayout => AbiMismatch::StructureLayout(AbiStructure::RuntimeApi),
            Self::BackendVTableLayout => AbiMismatch::StructureLayout(AbiStructure::BackendVTable),
        }
    }
}

pub fn mismatch_is_rejected_before_attach(fixture: AbiMismatchFixture) -> bool {
    let runtime = Runtime::new().expect("test runtime");
    let mut info = AbiInfo::query_linked().expect("linked ABI");
    info.corrupt(fixture.mismatch());
    let diagnostics = Arc::new(AtomicUsize::new(0));
    let diagnostic_count = Arc::clone(&diagnostics);
    let config = JitConfig::builder()
        .diagnostic_callback(move |diagnostic| {
            if matches!(diagnostic.kind(), JitDiagnosticKind::AbiMismatch(_)) {
                diagnostic_count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .unwrap();
    let rejected = matches!(
        Jit::attach_with_info(&runtime, config, info),
        Err(JitError::Abi(_))
    ) && diagnostics.load(Ordering::SeqCst) == 1;
    if !rejected {
        return false;
    }

    // A valid attachment succeeding immediately afterward proves the rejected
    // fixture never stored its vtable in the runtime.
    Jit::attach(&runtime, JitConfig::default()).is_ok()
}
