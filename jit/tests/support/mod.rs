use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use rquickjs_core::runtime::{JitBackend, JitBackendAttachError, RuntimeJitGuard};
use rquickjs_core::{context::EvalOptions, Context, Runtime, Value};

use crate::abi::{AbiInfo, AbiMismatch, AbiStructure};
use crate::bytecode::{
    CompileSnapshot, DeoptPoint, OsrPoint, RuntimeConstants, SnapshotStatus, VerifierMetadata,
};
use crate::{Jit, JitConfig, JitDiagnosticKind, JitError};

pub use crate::bytecode::decode_raw;

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
    BackendVTableLayout,
}

impl AbiMismatchFixture {
    pub const ALL: [Self; 12] = [
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
