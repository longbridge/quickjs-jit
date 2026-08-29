//! Safe ownership bridge for the versioned QuickJS JIT backend ABI.

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    sync::{Arc, Weak as ArcWeak},
};
use core::{
    cell::UnsafeCell,
    ffi::c_void,
    fmt, mem,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{qjs, Ctx, Function};

use super::{Runtime, WeakRuntime};

type FunctionKey = (u64, u64);

struct RetainedFunction {
    ctx: NonNull<qjs::JSContext>,
    function: qjs::JSValue,
}

impl Drop for RetainedFunction {
    fn drop(&mut self) {
        unsafe {
            qjs::JS_FreeValue(self.ctx.as_ptr(), self.function);
            qjs::JS_FreeContext(self.ctx.as_ptr());
        }
    }
}

struct FunctionRegistryState {
    rt: NonNull<qjs::JSRuntime>,
    functions: BTreeMap<FunctionKey, RetainedFunction>,
}

struct FunctionRegistryCell {
    attached: AtomicBool,
    state: UnsafeCell<FunctionRegistryState>,
}

// Registry state is accessed only by APIs that borrow a `Ctx` for this exact
// runtime, or by QuickJS callbacks. Both paths execute while the runtime lock
// is held. The atomic attached flag is the sole worker-readable state.
unsafe impl Send for FunctionRegistryCell {}
unsafe impl Sync for FunctionRegistryCell {}

struct FunctionRegistryOwner(Arc<FunctionRegistryCell>);

impl FunctionRegistryOwner {
    fn new(rt: NonNull<qjs::JSRuntime>) -> Self {
        Self(Arc::new(FunctionRegistryCell {
            attached: AtomicBool::new(true),
            state: UnsafeCell::new(FunctionRegistryState {
                rt,
                functions: BTreeMap::new(),
            }),
        }))
    }

    fn handle(&self) -> JitFunctionRegistry {
        JitFunctionRegistry {
            inner: Arc::downgrade(&self.0),
        }
    }

    unsafe fn retire(&self, id: u64, generation: u64) {
        let retained = unsafe {
            (&mut *self.0.state.get())
                .functions
                .remove(&(id, generation))
        };
        drop(retained);
    }

    unsafe fn clear(&self) {
        self.0.attached.store(false, Ordering::Release);
        unsafe { (&mut *self.0.state.get()).functions.clear() };
    }
}

/// A weak, runtime-thread-only handle to functions retained by an attached JIT backend.
///
/// The registry duplicates a function and its context, which transitively
/// retains the function's constant pool without storing a strong [`Runtime`]
/// reference. Retained QuickJS values are released by the owning raw runtime
/// during backend detachment, before the backend allocation and runtime are
/// freed. This handle contains no raw QuickJS pointer and does not keep that
/// owner alive.
#[derive(Clone)]
pub struct JitFunctionRegistry {
    inner: ArcWeak<FunctionRegistryCell>,
}

impl JitFunctionRegistry {
    /// Retains one source function and its transitive runtime constants.
    ///
    /// This must be called from a `Context::with` callback for the runtime to
    /// which this registry is attached. Replacing an existing key releases
    /// the old duplicate on that same runtime thread.
    pub fn retain_function<'js>(
        &self,
        ctx: &Ctx<'js>,
        function: &Function<'js>,
        id: u64,
        generation: u64,
    ) -> Result<(), JitFunctionRegistryError> {
        let registry = self
            .inner
            .upgrade()
            .ok_or(JitFunctionRegistryError::Detached)?;
        if !registry.attached.load(Ordering::Acquire) {
            return Err(JitFunctionRegistryError::Detached);
        }

        let ctx_ptr = ctx.as_raw();
        let state = unsafe { &*registry.state.get() };
        if unsafe { qjs::JS_GetRuntime(ctx_ptr.as_ptr()) } != state.rt.as_ptr() {
            return Err(JitFunctionRegistryError::WrongRuntime);
        }

        let retained_ctx = NonNull::new(unsafe { qjs::JS_DupContext(ctx_ptr.as_ptr()) })
            .ok_or(JitFunctionRegistryError::Detached)?;
        let retained = RetainedFunction {
            ctx: retained_ctx,
            function: unsafe { qjs::JS_DupValue(ctx_ptr.as_ptr(), function.as_value().as_raw()) },
        };
        let replaced = unsafe {
            (&mut *registry.state.get())
                .functions
                .insert((id, generation), retained)
        };
        drop(replaced);
        Ok(())
    }

    /// Returns the number of retained source functions while on the runtime thread.
    pub fn retained_len<'js>(&self, ctx: &Ctx<'js>) -> Result<usize, JitFunctionRegistryError> {
        let registry = self
            .inner
            .upgrade()
            .ok_or(JitFunctionRegistryError::Detached)?;
        if !registry.attached.load(Ordering::Acquire) {
            return Err(JitFunctionRegistryError::Detached);
        }
        let state = unsafe { &*registry.state.get() };
        if unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) } != state.rt.as_ptr() {
            return Err(JitFunctionRegistryError::WrongRuntime);
        }
        Ok(state.functions.len())
    }

    /// Whether the RawRuntime-owned registry still exists and is attached.
    ///
    /// This status check does not expose or access any retained QuickJS value.
    pub fn is_attached(&self) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|registry| registry.attached.load(Ordering::Acquire))
    }
}

impl fmt::Debug for JitFunctionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JitFunctionRegistry")
            .field("attached", &self.is_attached())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JitFunctionRegistryError {
    Detached,
    WrongRuntime,
}

impl fmt::Display for JitFunctionRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detached => f.write_str("the JIT function registry is detached"),
            Self::WrongRuntime => f.write_str("the context belongs to a different runtime"),
        }
    }
}

/// A backend registered with one QuickJS runtime.
///
/// # Safety
///
/// Implementations must obey the ownership contracts of `quickjs-jit.h`, must
/// not unwind through a callback, and must not retain borrowed callback
/// arguments beyond the call. Every nonempty entry handle must carry a pin
/// that keeps its code and metadata alive until `release_entry`; nonzero PCs
/// may be published only for verifier-approved OSR states. QuickJS invokes
/// callbacks only while the owning runtime is locked.
pub unsafe trait JitBackend: Send + 'static {
    /// Receives a weak runtime-thread registry handle during attachment.
    ///
    /// This Rust-only hook runs while the runtime lock is held. The handle may
    /// be sent to a coordinator, but retained QuickJS values can be accessed
    /// only through methods that require a same-runtime [`Ctx`].
    fn runtime_attached(&mut self, _registry: JitFunctionRegistry) {}

    fn record_hot(&mut self, _event: &qjs::JSJitHotEvent) -> u32 {
        0
    }

    fn submit_snapshot(&mut self, _snapshot: *mut qjs::JSJitFunctionSnapshot) {}

    fn acquire_entry(&mut self, _id: u64, _generation: u64, _pc: u32) -> qjs::JSJitEntryHandle {
        qjs::JSJitEntryHandle {
            struct_size: mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: None,
            pin: ptr::null_mut(),
        }
    }

    fn release_entry(&mut self, _entry: qjs::JSJitEntryHandle) {}

    fn runtime_detach(&mut self) {}

    fn function_retire(&mut self, _id: u64, _generation: u64) {}

    fn memory_used(&self) -> usize {
        0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JitBackendAttachError {
    AlreadyAttached,
    InvalidVTable,
    EngineRejected,
}

impl fmt::Display for JitBackendAttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyAttached => f.write_str("a JIT backend is already attached"),
            Self::InvalidVTable => f.write_str("the JIT backend vtable is incompatible"),
            Self::EngineRejected => f.write_str("QuickJS rejected the JIT backend"),
        }
    }
}

pub(super) struct BackendState {
    backend: Box<dyn JitBackend>,
    registry: FunctionRegistryOwner,
}

impl BackendState {
    pub(super) fn new(rt: NonNull<qjs::JSRuntime>, mut backend: Box<dyn JitBackend>) -> Self {
        let registry = FunctionRegistryOwner::new(rt);
        backend.runtime_attached(registry.handle());
        Self { backend, registry }
    }

    pub(super) fn as_opaque(&mut self) -> *mut c_void {
        (self as *mut Self).cast()
    }

    unsafe fn from_opaque<'a>(opaque: *mut c_void) -> &'a mut Self {
        debug_assert!(!opaque.is_null());
        unsafe { &mut *opaque.cast() }
    }
}

unsafe extern "C" fn record_hot(opaque: *mut c_void, event: *const qjs::JSJitHotEvent) -> u32 {
    if event.is_null() {
        return 0;
    }
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.record_hot(unsafe { &*event })
}

unsafe extern "C" fn submit_snapshot(
    opaque: *mut c_void,
    snapshot: *mut qjs::JSJitFunctionSnapshot,
) {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.submit_snapshot(snapshot);
}

unsafe extern "C" fn acquire_entry(
    opaque: *mut c_void,
    id: u64,
    generation: u64,
    pc: u32,
) -> qjs::JSJitEntryHandle {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.acquire_entry(id, generation, pc)
}

unsafe extern "C" fn release_entry(opaque: *mut c_void, entry: qjs::JSJitEntryHandle) {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.release_entry(entry);
}

unsafe extern "C" fn runtime_detach(opaque: *mut c_void, _rt: *mut qjs::JSRuntime) {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.runtime_detach();
    unsafe { state.registry.clear() };
}

unsafe extern "C" fn function_retire(opaque: *mut c_void, id: u64, generation: u64) {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state.backend.function_retire(id, generation);
    unsafe { state.registry.retire(id, generation) };
}

unsafe extern "C" fn memory_used(opaque: *mut c_void) -> qjs::size_t {
    let state = unsafe { BackendState::from_opaque(opaque) };
    state
        .backend
        .memory_used()
        .try_into()
        .unwrap_or(qjs::size_t::MAX)
}

static BACKEND_VTABLE: qjs::JSJitBackendVTable = qjs::JSJitBackendVTable {
    struct_size: mem::size_of::<qjs::JSJitBackendVTable>() as u32,
    record_hot: Some(record_hot),
    submit_snapshot: Some(submit_snapshot),
    acquire_entry: Some(acquire_entry),
    release_entry: Some(release_entry),
    runtime_detach: Some(runtime_detach),
    function_retire: Some(function_retire),
    memory_used: Some(memory_used),
};

/// Owns the token for one backend allocation stored by the raw runtime.
pub struct RuntimeJitGuard {
    runtime: WeakRuntime,
    token: u64,
}

impl RuntimeJitGuard {
    pub fn attach<B>(runtime: &Runtime, backend: B) -> Result<Self, JitBackendAttachError>
    where
        B: JitBackend,
    {
        let token = {
            let mut raw = runtime.inner.lock();
            unsafe { raw.attach_jit_backend(&BACKEND_VTABLE, Box::new(backend))? }
        };

        Ok(Self {
            runtime: runtime.weak(),
            token,
        })
    }
}

impl fmt::Debug for RuntimeJitGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeJitGuard").finish_non_exhaustive()
    }
}

impl Drop for RuntimeJitGuard {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.try_ref() else {
            // RawRuntime::drop already forced detachment and released the
            // backend allocation associated with this token.
            return;
        };
        let detached = unsafe { runtime.inner.lock().detach_jit_backend(self.token) };

        debug_assert!(detached.is_ok(), "QuickJS rejected JIT backend detach");
    }
}
