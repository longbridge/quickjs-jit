//! Cranelift lowering and W^X publication for Tier 1 pure frame operations.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[cfg(feature = "test-support")]
use std::sync::Mutex;

use cranelift_codegen::{
    binemit::Reloc as CraneliftReloc,
    control::ControlPlane,
    ir::{
        condcodes::{FloatCC, IntCC},
        types, AbiParam, ArgumentPurpose, Block, Function, InstBuilder, MemFlags, Signature,
        SourceLoc, Value,
    },
    isa::{unwind::UnwindInfo as CraneliftUnwindInfo, OwnedTargetIsa, TargetIsa},
    settings::{self, Configurable},
    Context, FinalizedRelocTarget,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use rquickjs_core::qjs;
use target_lexicon::{Endianness, Triple};

use crate::{
    bytecode::VerifiedFunction,
    code_cache::{
        CodeAllocation, CompiledArtifact, FrameState as ArtifactFrameState, Relocation,
        RelocationKind, RelocationTarget, StackMap, UnwindKind, UnwindMetadata,
    },
    ir::{BaselineIr, BinaryOp, FrameSlot, FrameStateId, IrOp, StackOp, TaggedValue, UnaryOp},
    platform::{CodeAllocator, CodeMemoryError, ExecutableCode},
    runtime::CompileRequest,
};

use super::{helpers::FrameLayout, CompileFailure, Compiler};

#[derive(Clone, Copy)]
struct Pair {
    payload: Value,
    tag: Value,
}

#[derive(Clone, Copy)]
struct PairVars {
    payload: Variable,
    tag: Variable,
}

/// Cranelift compiler configured for one explicit target ISA.
#[derive(Clone)]
pub struct BaselineCompiler {
    isa: OwnedTargetIsa,
    host_publishable: bool,
}

impl BaselineCompiler {
    pub fn host() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("opt_level", "speed")
            .expect("Cranelift opt_level setting");
        let flags = settings::Flags::new(flag_builder);
        let isa = cranelift_native::builder()
            .expect("host architecture is supported by Cranelift")
            .finish(flags)
            .expect("host ISA settings are valid");
        Self {
            isa,
            host_publishable: true,
        }
    }

    pub fn new(isa: OwnedTargetIsa) -> Self {
        let host_publishable = isa_is_host_compatible(&*isa);
        Self {
            isa,
            host_publishable,
        }
    }

    pub fn isa(&self) -> &dyn TargetIsa {
        &*self.isa
    }

    /// Compiles verified bytecode without allocating executable memory.
    pub fn compile(&self, function: &VerifiedFunction) -> Result<RelocatableCode, CompileFailure> {
        if self.isa.triple().pointer_width().map(|width| width.bits()) != Ok(64)
            || self.isa.triple().endianness() != Ok(Endianness::Little)
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let pointer_type = self.isa.pointer_type();
        let layout = FrameLayout::validated(
            u8::try_from(pointer_type.bytes()).map_err(|_| CompileFailure::InvalidArtifact)?,
        )?;
        let ir = BaselineIr::translate(function)?;
        let mut signature = Signature::new(self.isa.default_call_conv());
        signature.params.push(AbiParam::special(
            pointer_type,
            ArgumentPurpose::StructReturn,
        ));
        signature.params.push(AbiParam::new(pointer_type));

        let mut clif = Function::with_name_signature(Default::default(), signature);
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut clif, &mut builder_context);
            lower_function(&mut builder, &ir, &*self.isa, layout)?;
            builder.seal_all_blocks();
            builder.finalize();
        }

        let clif_text = clif.display().to_string();
        let function_parameters = clif.params.clone();
        let mut context = Context::for_function(clif);
        let compiled = context
            .compile(&*self.isa, &mut ControlPlane::default())
            .map_err(|_| CompileFailure::InvalidArtifact)?;
        let unwind_info = compiled
            .create_unwind_info(&*self.isa)
            .map_err(|_| CompileFailure::InvalidArtifact)?
            .ok_or(CompileFailure::InvalidArtifact)?;
        let unwind_metadata = Some(retain_unwind_metadata(&unwind_info, compiled.frame_size)?);
        let native_unwind = NativeUnwindPlan::new(unwind_info, &*self.isa)?;
        let bytes = compiled.code_buffer().to_vec();
        let mut code_offsets = BTreeMap::new();
        for location in compiled.buffer.get_srclocs_sorted() {
            if !location.loc.is_default() && location.start < location.end {
                code_offsets
                    .entry(location.loc.bits())
                    .or_insert(location.start);
            }
        }
        let relocations = compiled
            .buffer
            .relocs()
            .iter()
            .map(|reloc| {
                let target = match &reloc.target {
                    FinalizedRelocTarget::Func(offset) => RelocationTarget::FunctionOffset(*offset),
                    FinalizedRelocTarget::ExternalName(name) => RelocationTarget::Symbol(
                        name.display(Some(&function_parameters))
                            .to_string()
                            .into_boxed_str(),
                    ),
                };
                Relocation::with_target(
                    reloc.offset,
                    relocation_kind(reloc.kind),
                    target,
                    reloc.addend,
                )
            })
            .collect();
        let mut distinct_state_offsets = BTreeSet::new();
        let frame_states: Vec<_> = ir
            .frame_states
            .iter()
            .enumerate()
            .map(|state| {
                let (state_index, state) = state;
                let slots = state
                    .slots
                    .iter()
                    .map(|slot| match *slot {
                        FrameSlot::Argument(index) => Ok(index),
                        FrameSlot::Local(index) => ir
                            .argument_count
                            .checked_add(index)
                            .ok_or(CompileFailure::ResourceLimit),
                        FrameSlot::Stack(index) => ir
                            .argument_count
                            .checked_add(ir.local_count)
                            .and_then(|base| base.checked_add(index))
                            .ok_or(CompileFailure::ResourceLimit),
                    })
                    .collect::<Result<_, _>>()?;
                let state_id =
                    FrameStateId::from_index(state_index).ok_or(CompileFailure::ResourceLimit)?;
                let code_offset = code_offsets
                    .get(&frame_state_source_loc(state_id)?.bits())
                    .copied()
                    .ok_or(CompileFailure::InvalidArtifact)?;
                if code_offset as usize >= bytes.len()
                    || !distinct_state_offsets.insert(code_offset)
                {
                    return Err(CompileFailure::InvalidArtifact);
                }
                Ok(ArtifactFrameState::new(code_offset, state.pc, slots))
            })
            .collect::<Result<_, _>>()?;
        let stack_maps = frame_states
            .iter()
            .map(|state| StackMap::new(state.code_offset, state.slots.to_vec()))
            .collect();
        Ok(RelocatableCode {
            bytes,
            relocations,
            unwind_metadata,
            native_unwind,
            stack_maps,
            frame_states,
            target: self.isa.triple().clone(),
            host_publishable: self.host_publishable,
            clif: clif_text,
        })
    }
}

impl Compiler for BaselineCompiler {
    fn compile(&self, request: CompileRequest) -> Result<CompiledArtifact, CompileFailure> {
        let code = BaselineCompiler::compile(self, request.snapshot())?;
        Ok(CompiledArtifact::from_parts(
            request.artifact_key(),
            CodeAllocation::inert(code.bytes),
            code.relocations,
            code.stack_maps,
            code.frame_states,
            Vec::new(),
        )
        .with_unwind_metadata(code.unwind_metadata))
    }
}

/// Relocatable Cranelift output. Publication is intentionally host-gated.
#[derive(Clone, Debug)]
pub struct RelocatableCode {
    bytes: Vec<u8>,
    relocations: Vec<Relocation>,
    unwind_metadata: Option<UnwindMetadata>,
    native_unwind: NativeUnwindPlan,
    stack_maps: Vec<StackMap>,
    frame_states: Vec<ArtifactFrameState>,
    target: Triple,
    host_publishable: bool,
    clif: String,
}

impl RelocatableCode {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn relocations(&self) -> &[Relocation] {
        &self.relocations
    }

    pub fn unwind_metadata(&self) -> Option<&UnwindMetadata> {
        self.unwind_metadata.as_ref()
    }

    pub fn stack_maps(&self) -> &[StackMap] {
        &self.stack_maps
    }

    pub fn frame_states(&self) -> &[ArtifactFrameState] {
        &self.frame_states
    }

    /// Textual CLIF retained for ABI and safe-point audits.
    pub fn clif(&self) -> &str {
        &self.clif
    }

    pub fn target(&self) -> &Triple {
        &self.target
    }

    pub fn publish(self) -> Result<PublishedBaselineCode, CodeMemoryError> {
        if !self.host_publishable || self.target != Triple::host() {
            return Err(CodeMemoryError::TargetIsaMismatch);
        }
        let mut bytes = self.bytes;
        let native_unwind = self.native_unwind.prepare(&mut bytes)?;
        let allocator = CodeAllocator::for_host()?;
        let mut writable = allocator.allocate(bytes.len())?;
        writable.write(0, &bytes)?;
        let base = writable.as_ptr() as usize as u64;
        let mut resolved = Vec::with_capacity(self.relocations.len());
        for relocation in &self.relocations {
            let resolved_relocation = relocation
                .resolve_with(|target| match target {
                    RelocationTarget::FunctionOffset(offset) => {
                        base.checked_add(u64::from(*offset))
                    }
                    RelocationTarget::Absolute(value) => Some(*value),
                    RelocationTarget::Symbol(_) => None,
                })
                .map_err(|_| CodeMemoryError::UnresolvedRelocationTarget)?;
            resolved.push(resolved_relocation);
        }
        writable.apply_relocations(&resolved)?;
        writable.declare_indirect_targets(&[0])?;
        let executable = writable.publish()?;
        let unwind_registration = native_unwind.register(&executable)?;
        Ok(PublishedBaselineCode::new(
            executable,
            unwind_registration,
            self.unwind_metadata,
            self.stack_maps,
            self.frame_states,
        ))
    }
}

/// Published Tier 1 code together with metadata pinned for its full RX lifetime.
#[derive(Clone, Debug)]
pub struct PublishedBaselineCode {
    allocation: Arc<PublishedBaselineAllocation>,
}

#[derive(Debug)]
struct PublishedBaselineAllocation {
    unwind_registration: Option<NativeUnwindRegistration>,
    executable: Option<ExecutableCode>,
    unwind_metadata: Option<UnwindMetadata>,
    stack_maps: Box<[StackMap]>,
    frame_states: Box<[ArtifactFrameState]>,
    #[cfg(feature = "test-support")]
    lifetime_events: Arc<Mutex<Vec<&'static str>>>,
}

impl PublishedBaselineCode {
    fn new(
        executable: ExecutableCode,
        unwind_registration: NativeUnwindRegistration,
        unwind_metadata: Option<UnwindMetadata>,
        stack_maps: Vec<StackMap>,
        frame_states: Vec<ArtifactFrameState>,
    ) -> Self {
        Self {
            allocation: Arc::new(PublishedBaselineAllocation {
                unwind_registration: Some(unwind_registration),
                executable: Some(executable),
                unwind_metadata,
                stack_maps: stack_maps.into_boxed_slice(),
                frame_states: frame_states.into_boxed_slice(),
                #[cfg(feature = "test-support")]
                lifetime_events: Arc::new(Mutex::new(Vec::new())),
            }),
        }
    }

    pub fn unwind_metadata(&self) -> Option<&UnwindMetadata> {
        self.allocation.unwind_metadata.as_ref()
    }

    pub fn len(&self) -> usize {
        self.allocation
            .executable
            .as_ref()
            .expect("published executable remains live while pinned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.allocation
            .executable
            .as_ref()
            .expect("published executable remains live while pinned")
            .as_ptr()
    }

    pub const fn is_writable(&self) -> bool {
        false
    }

    pub fn stack_maps(&self) -> &[StackMap] {
        &self.allocation.stack_maps
    }

    pub fn frame_states(&self) -> &[ArtifactFrameState] {
        &self.allocation.frame_states
    }

    pub fn unwind_is_registered(&self) -> bool {
        self.allocation.unwind_registration.is_some()
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn lifetime_probe(&self) -> PublishedLifetimeProbe {
        PublishedLifetimeProbe(Arc::clone(&self.allocation.lifetime_events))
    }
}

impl Drop for PublishedBaselineAllocation {
    fn drop(&mut self) {
        drop(self.unwind_registration.take());
        #[cfg(feature = "test-support")]
        self.lifetime_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push("unwind_deregistered");

        drop(self.executable.take());
        #[cfg(feature = "test-support")]
        self.lifetime_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push("executable_released");
    }
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct PublishedLifetimeProbe(Arc<Mutex<Vec<&'static str>>>);

#[cfg(feature = "test-support")]
impl PublishedLifetimeProbe {
    pub fn events(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn retain_unwind_metadata(
    unwind: &CraneliftUnwindInfo,
    frame_size: u32,
) -> Result<UnwindMetadata, CompileFailure> {
    let kind = match &unwind {
        CraneliftUnwindInfo::SystemV(_) => UnwindKind::SystemV,
        CraneliftUnwindInfo::WindowsX64(_) => UnwindKind::WindowsX64,
        CraneliftUnwindInfo::WindowsArm64(_) => UnwindKind::WindowsArm64,
        _ => return Err(CompileFailure::InvalidArtifact),
    };
    let encoding = postcard::to_allocvec(unwind).map_err(|_| CompileFailure::InvalidArtifact)?;
    Ok(UnwindMetadata::new(kind, frame_size, encoding))
}

#[derive(Clone, Debug)]
struct NativeUnwindPlan {
    info: CraneliftUnwindInfo,
    systemv_cie: Option<gimli::write::CommonInformationEntry>,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    windows_function_len: Option<u32>,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    windows_unwind_offset: Option<u32>,
}

impl NativeUnwindPlan {
    fn new(info: CraneliftUnwindInfo, isa: &dyn TargetIsa) -> Result<Self, CompileFailure> {
        let systemv_cie = if matches!(info, CraneliftUnwindInfo::SystemV(_)) {
            Some(
                isa.create_systemv_cie()
                    .ok_or(CompileFailure::InvalidArtifact)?,
            )
        } else {
            None
        };
        Ok(Self {
            info,
            systemv_cie,
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            windows_function_len: None,
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            windows_unwind_offset: None,
        })
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn prepare(mut self, bytes: &mut Vec<u8>) -> Result<Self, CodeMemoryError> {
        let CraneliftUnwindInfo::WindowsX64(info) = &self.info else {
            return Err(CodeMemoryError::UnwindRegistrationUnsupported);
        };
        let function_len = u32::try_from(bytes.len()).map_err(|_| CodeMemoryError::SizeOverflow)?;
        let aligned_len = bytes
            .len()
            .checked_add(3)
            .map(|len| len & !3)
            .ok_or(CodeMemoryError::SizeOverflow)?;
        bytes.resize(aligned_len, 0);
        let unwind_offset =
            u32::try_from(aligned_len).map_err(|_| CodeMemoryError::SizeOverflow)?;
        let unwind_end = aligned_len
            .checked_add(info.emit_size())
            .ok_or(CodeMemoryError::SizeOverflow)?;
        bytes.resize(unwind_end, 0);
        info.emit(&mut bytes[aligned_len..unwind_end]);
        self.windows_function_len = Some(function_len);
        self.windows_unwind_offset = Some(unwind_offset);
        Ok(self)
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    fn prepare(self, _bytes: &mut Vec<u8>) -> Result<Self, CodeMemoryError> {
        Err(CodeMemoryError::UnwindRegistrationUnsupported)
    }

    #[cfg(not(all(
        target_os = "windows",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    fn prepare(self, _bytes: &mut Vec<u8>) -> Result<Self, CodeMemoryError> {
        Ok(self)
    }

    fn register(
        self,
        executable: &ExecutableCode,
    ) -> Result<NativeUnwindRegistration, CodeMemoryError> {
        #[cfg(all(
            any(target_os = "linux", target_os = "macos"),
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let CraneliftUnwindInfo::SystemV(info) = self.info {
            use gimli::{
                write::{Address, EhFrame, EndianVec, FrameTable},
                LittleEndian,
            };

            let cie = self
                .systemv_cie
                .ok_or(CodeMemoryError::UnwindRegistrationUnsupported)?;
            let mut table = FrameTable::default();
            let cie = table.add_cie(cie);
            table.add_fde(
                cie,
                info.to_fde(Address::Constant(executable.as_ptr() as usize as u64)),
            );
            let mut eh_frame = EhFrame::from(EndianVec::new(LittleEndian));
            table.write_eh_frame(&mut eh_frame).map_err(|_| {
                CodeMemoryError::UnwindRegistrationFailed {
                    operation: "encode System V .eh_frame",
                }
            })?;
            let mut bytes = eh_frame.0.into_vec();
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
            let bytes = bytes.into_boxed_slice();
            let registration_offset = systemv_registration_offset(&bytes)?;
            unsafe {
                systemv_register_frame(bytes.as_ptr().add(registration_offset).cast());
            }
            return Ok(NativeUnwindRegistration {
                systemv_eh_frame: Some(bytes),
                systemv_registration_offset: registration_offset,
            });
        }

        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        if matches!(&self.info, CraneliftUnwindInfo::WindowsX64(_)) {
            use windows_sys::Win32::System::Diagnostics::Debug::{
                RtlAddFunctionTable, IMAGE_RUNTIME_FUNCTION_ENTRY, IMAGE_RUNTIME_FUNCTION_ENTRY_0,
            };

            let mut function_table = Box::new(IMAGE_RUNTIME_FUNCTION_ENTRY {
                BeginAddress: 0,
                EndAddress: self
                    .windows_function_len
                    .ok_or(CodeMemoryError::UnwindRegistrationUnsupported)?,
                Anonymous: IMAGE_RUNTIME_FUNCTION_ENTRY_0 {
                    UnwindInfoAddress: self
                        .windows_unwind_offset
                        .ok_or(CodeMemoryError::UnwindRegistrationUnsupported)?,
                },
            });
            let registered = unsafe {
                RtlAddFunctionTable(
                    function_table.as_mut(),
                    1,
                    executable.as_ptr() as usize as u64,
                )
            };
            if !registered {
                return Err(CodeMemoryError::UnwindRegistrationFailed {
                    operation: "register Windows x64 function table",
                });
            }
            return Ok(NativeUnwindRegistration {
                systemv_eh_frame: None,
                systemv_registration_offset: 0,
                windows_function_table: Some(function_table),
            });
        }

        let _ = executable;
        Err(CodeMemoryError::UnwindRegistrationUnsupported)
    }
}

struct NativeUnwindRegistration {
    systemv_eh_frame: Option<Box<[u8]>>,
    systemv_registration_offset: usize,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    windows_function_table:
        Option<Box<windows_sys::Win32::System::Diagnostics::Debug::IMAGE_RUNTIME_FUNCTION_ENTRY>>,
}

impl std::fmt::Debug for NativeUnwindRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeUnwindRegistration")
            .field(
                "systemv_eh_frame_len",
                &self.systemv_eh_frame.as_ref().map(|bytes| bytes.len()),
            )
            .field(
                "systemv_registration_offset",
                &self.systemv_registration_offset,
            )
            .finish_non_exhaustive()
    }
}

impl Drop for NativeUnwindRegistration {
    fn drop(&mut self) {
        #[cfg(all(
            any(target_os = "linux", target_os = "macos"),
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Some(bytes) = self.systemv_eh_frame.as_ref() {
            unsafe {
                systemv_deregister_frame(
                    bytes.as_ptr().add(self.systemv_registration_offset).cast(),
                );
            }
        }

        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        if let Some(function_table) = self.windows_function_table.as_ref() {
            use windows_sys::Win32::System::Diagnostics::Debug::RtlDeleteFunctionTable;

            unsafe {
                let _ = RtlDeleteFunctionTable(function_table.as_ref());
            }
        }
    }
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn systemv_registration_offset(_eh_frame: &[u8]) -> Result<usize, CodeMemoryError> {
    // GCC's frame API consumes a complete, zero-terminated .eh_frame section.
    Ok(0)
}

#[cfg(all(
    target_os = "macos",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn systemv_registration_offset(eh_frame: &[u8]) -> Result<usize, CodeMemoryError> {
    // Apple's libunwind frame API consumes one FDE rather than the section's CIE.
    let cie_length = eh_frame
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or(CodeMemoryError::UnwindRegistrationFailed {
            operation: "locate the System V FDE",
        })?;
    let fde_offset =
        4_usize
            .checked_add(cie_length)
            .ok_or(CodeMemoryError::UnwindRegistrationFailed {
                operation: "locate the System V FDE",
            })?;
    if fde_offset
        .checked_add(4)
        .is_none_or(|end| end > eh_frame.len())
    {
        return Err(CodeMemoryError::UnwindRegistrationFailed {
            operation: "locate the System V FDE",
        });
    }
    Ok(fde_offset)
}

#[cfg(all(
    target_os = "linux",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[link(name = "gcc_s")]
unsafe extern "C" {
    #[link_name = "__register_frame"]
    fn systemv_register_frame(begin: *const std::ffi::c_void);
    #[link_name = "__deregister_frame"]
    fn systemv_deregister_frame(begin: *const std::ffi::c_void);
}

#[cfg(all(
    target_os = "macos",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
unsafe extern "C" {
    #[link_name = "__register_frame"]
    fn systemv_register_frame(begin: *const std::ffi::c_void);
    #[link_name = "__deregister_frame"]
    fn systemv_deregister_frame(begin: *const std::ffi::c_void);
}

fn isa_is_host_compatible(target: &dyn TargetIsa) -> bool {
    if target.triple() != &Triple::host() {
        return false;
    }
    let Ok(builder) = cranelift_native::builder() else {
        return false;
    };
    let Ok(host) = builder.finish(settings::Flags::new(settings::builder())) else {
        return false;
    };
    if target.is_branch_protection_enabled() && !host.is_branch_protection_enabled() {
        return false;
    }
    let host_flags: BTreeMap<_, _> = host
        .isa_flags()
        .into_iter()
        .map(|value| (value.name, (value.as_bool(), value.value_string())))
        .collect();
    target.isa_flags().into_iter().all(|value| {
        let Some((host_bool, host_value)) = host_flags.get(value.name) else {
            return false;
        };
        match value.as_bool() {
            Some(false) => true,
            Some(true) => *host_bool == Some(true),
            None => value.value_string() == *host_value,
        }
    })
}

fn relocation_kind(kind: CraneliftReloc) -> RelocationKind {
    match kind {
        CraneliftReloc::Abs4 => RelocationKind::Abs4,
        CraneliftReloc::Abs8 => RelocationKind::Abs8,
        CraneliftReloc::X86PCRel4 => RelocationKind::X86PCRel4,
        CraneliftReloc::X86CallPCRel4 => RelocationKind::X86CallPCRel4,
        CraneliftReloc::X86CallPLTRel4 => RelocationKind::X86CallPLTRel4,
        CraneliftReloc::X86GOTPCRel4 => RelocationKind::X86GOTPCRel4,
        CraneliftReloc::X86SecRel => RelocationKind::X86SecRel,
        CraneliftReloc::Arm32Call => RelocationKind::Arm32Call,
        CraneliftReloc::Arm64Call => RelocationKind::Arm64Call,
        CraneliftReloc::S390xPCRel32Dbl => RelocationKind::S390xPCRel32Dbl,
        CraneliftReloc::S390xPLTRel32Dbl => RelocationKind::S390xPLTRel32Dbl,
        CraneliftReloc::ElfX86_64TlsGd => RelocationKind::ElfX86_64TlsGd,
        CraneliftReloc::MachOX86_64Tlv => RelocationKind::MachOX86_64Tlv,
        CraneliftReloc::MachOAarch64TlsAdrPage21 => RelocationKind::MachOAarch64TlsAdrPage21,
        CraneliftReloc::MachOAarch64TlsAdrPageOff12 => RelocationKind::MachOAarch64TlsAdrPageOff12,
        CraneliftReloc::Aarch64TlsDescAdrPage21 => RelocationKind::Aarch64TlsDescAdrPage21,
        CraneliftReloc::Aarch64TlsDescLd64Lo12 => RelocationKind::Aarch64TlsDescLd64Lo12,
        CraneliftReloc::Aarch64TlsDescAddLo12 => RelocationKind::Aarch64TlsDescAddLo12,
        CraneliftReloc::Aarch64TlsDescCall => RelocationKind::Aarch64TlsDescCall,
        CraneliftReloc::Aarch64AdrGotPage21 => RelocationKind::Aarch64AdrGotPage21,
        CraneliftReloc::Aarch64Ld64GotLo12Nc => RelocationKind::Aarch64Ld64GotLo12Nc,
        CraneliftReloc::RiscvCallPlt => RelocationKind::RiscvCallPlt,
        CraneliftReloc::RiscvTlsGdHi20 => RelocationKind::RiscvTlsGdHi20,
        CraneliftReloc::RiscvPCRelLo12I => RelocationKind::RiscvPCRelLo12I,
        CraneliftReloc::RiscvGotHi20 => RelocationKind::RiscvGotHi20,
        CraneliftReloc::S390xTlsGd64 => RelocationKind::S390xTlsGd64,
        CraneliftReloc::S390xTlsGdCall => RelocationKind::S390xTlsGdCall,
        CraneliftReloc::PulleyCallIndirectHost => RelocationKind::PulleyCallIndirectHost,
    }
}

fn frame_state_source_loc(state: FrameStateId) -> Result<SourceLoc, CompileFailure> {
    let bits = u32::try_from(state.index())
        .ok()
        .and_then(|index| index.checked_add(1))
        .filter(|bits| *bits != u32::MAX)
        .ok_or(CompileFailure::ResourceLimit)?;
    Ok(SourceLoc::new(bits))
}

fn lower_function(
    builder: &mut FunctionBuilder<'_>,
    ir: &BaselineIr,
    isa: &dyn TargetIsa,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let pointer_type = isa.pointer_type();
    let blocks: BTreeMap<u32, Block> = ir
        .blocks
        .iter()
        .map(|block| (block.start_pc, builder.create_block()))
        .collect();
    let retry = builder.create_block();
    let entry = *blocks.get(&0).ok_or(CompileFailure::InvalidArtifact)?;
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let params = builder.block_params(entry);
    if params.len() != 2 {
        return Err(CompileFailure::InvalidArtifact);
    }
    let sret = params[0];
    let frame = params[1];

    let mut next_variable = 0_u32;
    let mut allocate_pair = || {
        let pair = PairVars {
            payload: Variable::from_u32(next_variable),
            tag: Variable::from_u32(next_variable + 1),
        };
        next_variable += 2;
        pair
    };
    let arguments: Vec<PairVars> = (0..ir.argument_count).map(|_| allocate_pair()).collect();
    let locals: Vec<PairVars> = (0..ir.local_count).map(|_| allocate_pair()).collect();
    let stack: Vec<PairVars> = (0..ir.max_stack_depth).map(|_| allocate_pair()).collect();
    for pair in arguments.iter().chain(&locals).chain(&stack) {
        builder.declare_var(pair.payload, types::I64);
        builder.declare_var(pair.tag, types::I64);
    }
    let flags = MemFlags::new();
    let arg_buf = builder
        .ins()
        .load(pointer_type, flags, frame, layout.arg_buf);
    let var_buf = builder
        .ins()
        .load(pointer_type, flags, frame, layout.var_buf);
    for (index, pair) in arguments.iter().copied().enumerate() {
        let value = load_jsvalue(builder, arg_buf, index, layout);
        define_pair(builder, pair, value);
    }
    for (index, pair) in locals.iter().copied().enumerate() {
        let value = load_jsvalue(builder, var_buf, index, layout);
        define_pair(builder, pair, value);
    }
    for pair in arguments.iter().chain(&locals).copied() {
        let value = use_pair(builder, pair);
        guard_non_refcounted(builder, value, retry);
    }
    guard_live_stack_non_refcounted(builder, frame, retry, pointer_type, layout);
    for pair in stack.iter().copied() {
        let value = constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
        define_pair(builder, pair, value);
    }

    let mut poll_signature = Signature::new(isa.default_call_conv());
    poll_signature.params.push(AbiParam::new(pointer_type));
    poll_signature.returns.push(AbiParam::new(types::I32));
    let poll_signature = builder.import_signature(poll_signature);

    for (block_index, block) in ir.blocks.iter().enumerate() {
        let clif_block = blocks[&block.start_pc];
        if clif_block != entry {
            builder.switch_to_block(clif_block);
        }
        let mut depth = block.stack_depth as usize;
        let mut terminated = false;
        for instruction in &block.instructions {
            builder.set_srcloc(
                instruction
                    .frame_state
                    .map(frame_state_source_loc)
                    .transpose()?
                    .unwrap_or_default(),
            );
            match instruction.op {
                IrOp::Poll { .. } => emit_poll(
                    builder,
                    frame,
                    sret,
                    poll_signature,
                    instruction.pc,
                    pointer_type,
                    layout,
                ),
                IrOp::OsrLabel { .. } => {
                    // A trapping-capable frame load is an observable native marker that
                    // survives lowering, giving this OSR state its own exact code range.
                    let _ = builder.ins().load(
                        pointer_type,
                        MemFlags::new(),
                        frame,
                        layout.bytecode_start,
                    );
                }
                IrOp::Nop => {}
                IrOp::Push(value) => {
                    let value = constant_pair(builder, value);
                    define_pair(builder, stack[depth], value);
                    depth += 1;
                }
                IrOp::GetArgument(index) => {
                    copy_pair(builder, stack[depth], arguments[index as usize]);
                    depth += 1;
                }
                IrOp::GetLocal(index) => {
                    copy_pair(builder, stack[depth], locals[index as usize]);
                    depth += 1;
                }
                IrOp::GetLocalChecked(index) => {
                    let value = use_pair(builder, locals[index as usize]);
                    let initialized = builder.ins().icmp_imm(
                        IntCC::NotEqual,
                        value.tag,
                        i64::from(qjs::JS_TAG_UNINITIALIZED),
                    );
                    guard(builder, initialized, retry);
                    define_pair(builder, stack[depth], value);
                    depth += 1;
                }
                IrOp::GetLocalPair => {
                    copy_pair(builder, stack[depth], locals[0]);
                    copy_pair(builder, stack[depth + 1], locals[1]);
                    depth += 2;
                }
                IrOp::PutArgument { index, keep } => {
                    let source = stack[depth - 1];
                    copy_pair(builder, arguments[index as usize], source);
                    if !keep {
                        depth -= 1;
                    }
                }
                IrOp::PutLocal { index, keep } => {
                    let source = stack[depth - 1];
                    copy_pair(builder, locals[index as usize], source);
                    if !keep {
                        depth -= 1;
                    }
                }
                IrOp::PutLocalChecked { index, initialize } => {
                    let current = use_pair(builder, locals[index as usize]);
                    let condition = builder.ins().icmp_imm(
                        if initialize {
                            IntCC::Equal
                        } else {
                            IntCC::NotEqual
                        },
                        current.tag,
                        i64::from(qjs::JS_TAG_UNINITIALIZED),
                    );
                    guard(builder, condition, retry);
                    let source = stack[depth - 1];
                    copy_pair(builder, locals[index as usize], source);
                    depth -= 1;
                }
                IrOp::SetLocalUninitialized(index) => {
                    let value = constant_pair(
                        builder,
                        TaggedValue::new(0, qjs::JS_TAG_UNINITIALIZED as i64),
                    );
                    define_pair(builder, locals[index as usize], value);
                }
                IrOp::Drop => depth -= 1,
                IrOp::Stack(operation) => {
                    apply_stack_operation(builder, &stack, &mut depth, operation)
                }
                IrOp::Unary(operation) => {
                    let value = use_pair(builder, stack[depth - 1]);
                    let result = emit_unary(builder, value, operation, retry);
                    define_pair(builder, stack[depth - 1], result);
                }
                IrOp::PostUnary(operation) => {
                    let value = use_pair(builder, stack[depth - 1]);
                    let result = emit_unary(builder, value, operation, retry);
                    define_pair(builder, stack[depth], result);
                    depth += 1;
                }
                IrOp::LocalUnary { index, op } => {
                    let value = use_pair(builder, locals[index as usize]);
                    let result = emit_unary(builder, value, op, retry);
                    define_pair(builder, locals[index as usize], result);
                }
                IrOp::AddLocal(index) => {
                    let left = use_pair(builder, locals[index as usize]);
                    let right = use_pair(builder, stack[depth - 1]);
                    let result = emit_binary(builder, left, right, BinaryOp::Add, retry);
                    define_pair(builder, locals[index as usize], result);
                    depth -= 1;
                }
                IrOp::Binary(operation) => {
                    let left = use_pair(builder, stack[depth - 2]);
                    let right = use_pair(builder, stack[depth - 1]);
                    let result = emit_binary(builder, left, right, operation, retry);
                    depth -= 1;
                    define_pair(builder, stack[depth - 1], result);
                }
                IrOp::Jump(target) => {
                    builder.ins().jump(blocks[&target], &[]);
                    terminated = true;
                }
                IrOp::Branch { target, when_true } => {
                    let condition = use_pair(builder, stack[depth - 1]);
                    depth -= 1;
                    let truthy = emit_truthy(builder, condition, retry);
                    let taken = if when_true {
                        truthy
                    } else {
                        builder.ins().bxor_imm(truthy, 1)
                    };
                    let fallthrough = ir
                        .blocks
                        .get(block_index + 1)
                        .ok_or(CompileFailure::InvalidArtifact)?
                        .start_pc;
                    builder
                        .ins()
                        .brif(taken, blocks[&target], &[], blocks[&fallthrough], &[]);
                    terminated = true;
                }
                IrOp::Return => {
                    let result = use_pair(builder, stack[depth - 1]);
                    let returnable = emit_returnable_tag(builder, result.tag);
                    guard(builder, returnable, retry);
                    store_jsvalue(builder, frame, layout.result, result, layout);
                    emit_exit(
                        builder,
                        sret,
                        qjs::JSJitExitKind_JS_JIT_EXIT_DONE,
                        None,
                        pointer_type,
                    );
                    terminated = true;
                }
                IrOp::ReturnUndefined => {
                    let result =
                        constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
                    store_jsvalue(builder, frame, layout.result, result, layout);
                    emit_exit(
                        builder,
                        sret,
                        qjs::JSJitExitKind_JS_JIT_EXIT_DONE,
                        None,
                        pointer_type,
                    );
                    terminated = true;
                }
            }
            if terminated {
                break;
            }
        }
        if !terminated {
            if let Some(next) = ir.blocks.get(block_index + 1) {
                builder.ins().jump(blocks[&next.start_pc], &[]);
            } else {
                emit_exit(
                    builder,
                    sret,
                    qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
                    None,
                    pointer_type,
                );
            }
        }
    }

    builder.set_srcloc(SourceLoc::default());
    builder.switch_to_block(retry);
    emit_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
        None,
        pointer_type,
    );
    Ok(())
}

fn load_jsvalue(
    builder: &mut FunctionBuilder<'_>,
    base: Value,
    index: usize,
    layout: FrameLayout,
) -> Pair {
    let offset = i32::try_from(index * 16).expect("verified frame index fits i32");
    let flags = MemFlags::new();
    Pair {
        payload: builder.ins().load(types::I64, flags, base, offset),
        tag: builder
            .ins()
            .load(types::I64, flags, base, offset + layout.value_tag),
    }
}

fn store_jsvalue(
    builder: &mut FunctionBuilder<'_>,
    base: Value,
    offset: i32,
    value: Pair,
    layout: FrameLayout,
) {
    let flags = MemFlags::new();
    builder.ins().store(flags, value.payload, base, offset);
    builder
        .ins()
        .store(flags, value.tag, base, offset + layout.value_tag);
}

fn constant_pair(builder: &mut FunctionBuilder<'_>, value: TaggedValue) -> Pair {
    Pair {
        payload: builder.ins().iconst(types::I64, value.payload as i64),
        tag: builder.ins().iconst(types::I64, value.tag),
    }
}

fn define_pair(builder: &mut FunctionBuilder<'_>, variables: PairVars, value: Pair) {
    builder.def_var(variables.payload, value.payload);
    builder.def_var(variables.tag, value.tag);
}

fn copy_pair(builder: &mut FunctionBuilder<'_>, destination: PairVars, source: PairVars) {
    let value = use_pair(builder, source);
    define_pair(builder, destination, value);
}

fn use_pair(builder: &mut FunctionBuilder<'_>, variables: PairVars) -> Pair {
    Pair {
        payload: builder.use_var(variables.payload),
        tag: builder.use_var(variables.tag),
    }
}

fn guard_non_refcounted(builder: &mut FunctionBuilder<'_>, value: Pair, retry: Block) {
    let negative = builder.ins().icmp_imm(IntCC::SignedLessThan, value.tag, 0);
    let at_or_after_first = builder.ins().icmp_imm(
        IntCC::SignedGreaterThanOrEqual,
        value.tag,
        i64::from(qjs::JS_TAG_FIRST),
    );
    let refcounted = builder.ins().band(negative, at_or_after_first);
    let immediate = builder.ins().bxor_imm(refcounted, 1);
    guard(builder, immediate, retry);
}

fn guard_live_stack_non_refcounted(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    retry: Block,
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) {
    let flags = MemFlags::new();
    let stack_base = builder
        .ins()
        .load(pointer_type, flags, frame, layout.stack_base);
    let stack_top = builder
        .ins()
        .load(pointer_type, flags, frame, layout.stack_top);
    let scan = builder.create_block();
    let inspect = builder.create_block();
    let continuation = builder.create_block();
    builder.append_block_param(scan, pointer_type);
    builder.ins().jump(scan, &[stack_base]);

    builder.switch_to_block(scan);
    let cursor = builder.block_params(scan)[0];
    let complete = builder.ins().icmp(IntCC::Equal, cursor, stack_top);
    builder
        .ins()
        .brif(complete, continuation, &[], inspect, &[cursor]);

    builder.append_block_param(inspect, pointer_type);
    builder.switch_to_block(inspect);
    let cursor = builder.block_params(inspect)[0];
    let tag = builder
        .ins()
        .load(types::I64, flags, cursor, layout.value_tag);
    let value = Pair {
        payload: builder.ins().iconst(types::I64, 0),
        tag,
    };
    let negative = builder.ins().icmp_imm(IntCC::SignedLessThan, value.tag, 0);
    let at_or_after_first = builder.ins().icmp_imm(
        IntCC::SignedGreaterThanOrEqual,
        value.tag,
        i64::from(qjs::JS_TAG_FIRST),
    );
    let refcounted = builder.ins().band(negative, at_or_after_first);
    let immediate = builder.ins().bxor_imm(refcounted, 1);
    let next = builder.ins().iadd_imm(cursor, 16);
    builder.ins().brif(immediate, scan, &[next], retry, &[]);

    builder.seal_block(inspect);
    builder.seal_block(scan);
    builder.seal_block(continuation);
    builder.switch_to_block(continuation);
}

fn emit_exit(
    builder: &mut FunctionBuilder<'_>,
    sret: Value,
    kind: qjs::JSJitExitKind,
    resume_pc: Option<Value>,
    pointer_type: cranelift_codegen::ir::Type,
) {
    let flags = MemFlags::new();
    let kind = builder.ins().iconst(types::I32, i64::from(kind));
    let zero32 = builder.ins().iconst(types::I32, 0);
    let zero_pointer = builder.ins().iconst(pointer_type, 0);
    builder.ins().store(flags, kind, sret, 0);
    builder.ins().store(flags, zero32, sret, 4);
    builder
        .ins()
        .store(flags, resume_pc.unwrap_or(zero_pointer), sret, 8);
    builder.ins().store(flags, zero_pointer, sret, 16);
    builder.ins().return_(&[]);
}

fn emit_poll(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    sret: Value,
    signature: cranelift_codegen::ir::SigRef,
    pc: u32,
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) {
    let flags = MemFlags::new();
    let api = builder
        .ins()
        .load(pointer_type, flags, frame, layout.runtime_api);
    let poll = builder.ins().load(pointer_type, flags, api, layout.poll);
    let call = builder.ins().call_indirect(signature, poll, &[frame]);
    let interrupted = builder.inst_results(call)[0];
    let interrupted = builder.ins().icmp_imm(IntCC::NotEqual, interrupted, 0);
    let interrupt = builder.create_block();
    let continuation = builder.create_block();
    builder
        .ins()
        .brif(interrupted, interrupt, &[], continuation, &[]);
    builder.seal_block(interrupt);
    builder.seal_block(continuation);
    builder.switch_to_block(interrupt);
    let bytecode = builder
        .ins()
        .load(pointer_type, flags, frame, layout.bytecode_start);
    let resume = builder.ins().iadd_imm(bytecode, i64::from(pc));
    emit_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_INTERRUPT,
        Some(resume),
        pointer_type,
    );
    builder.switch_to_block(continuation);
}

fn guard(builder: &mut FunctionBuilder<'_>, condition: Value, retry: Block) {
    let continuation = builder.create_block();
    builder.ins().brif(condition, continuation, &[], retry, &[]);
    builder.seal_block(continuation);
    builder.switch_to_block(continuation);
}

fn tag_is(builder: &mut FunctionBuilder<'_>, tag: Value, expected: i32) -> Value {
    builder
        .ins()
        .icmp_imm(IntCC::Equal, tag, i64::from(expected))
}

fn emit_returnable_tag(builder: &mut FunctionBuilder<'_>, tag: Value) -> Value {
    let int = tag_is(builder, tag, qjs::JS_TAG_INT);
    let boolean = tag_is(builder, tag, qjs::JS_TAG_BOOL);
    let null = tag_is(builder, tag, qjs::JS_TAG_NULL);
    let undefined = tag_is(builder, tag, qjs::JS_TAG_UNDEFINED);
    let short_big_int = tag_is(builder, tag, qjs::JS_TAG_SHORT_BIG_INT);
    let float = tag_is(builder, tag, qjs::JS_TAG_FLOAT64);
    let result = builder.ins().bor(int, boolean);
    let result = builder.ins().bor(result, null);
    let result = builder.ins().bor(result, undefined);
    let result = builder.ins().bor(result, short_big_int);
    builder.ins().bor(result, float)
}

fn emit_numeric(builder: &mut FunctionBuilder<'_>, value: Pair, retry: Block) -> (Value, Value) {
    let is_int = tag_is(builder, value.tag, qjs::JS_TAG_INT);
    let is_float = tag_is(builder, value.tag, qjs::JS_TAG_FLOAT64);
    let supported = builder.ins().bor(is_int, is_float);
    guard(builder, supported, retry);
    let int = builder.ins().ireduce(types::I32, value.payload);
    let int_float = builder.ins().fcvt_from_sint(types::F64, int);
    let raw_float = builder
        .ins()
        .bitcast(types::F64, MemFlags::new(), value.payload);
    let number = builder.ins().select(is_int, int_float, raw_float);
    (is_int, number)
}

fn pair_from_bool(builder: &mut FunctionBuilder<'_>, value: Value) -> Pair {
    Pair {
        payload: builder.ins().uextend(types::I64, value),
        tag: builder
            .ins()
            .iconst(types::I64, i64::from(qjs::JS_TAG_BOOL)),
    }
}

fn emit_truthy(builder: &mut FunctionBuilder<'_>, value: Pair, retry: Block) -> Value {
    let is_int = tag_is(builder, value.tag, qjs::JS_TAG_INT);
    let is_bool = tag_is(builder, value.tag, qjs::JS_TAG_BOOL);
    let is_null = tag_is(builder, value.tag, qjs::JS_TAG_NULL);
    let is_undefined = tag_is(builder, value.tag, qjs::JS_TAG_UNDEFINED);
    let is_float = tag_is(builder, value.tag, qjs::JS_TAG_FLOAT64);
    let scalar = builder.ins().bor(is_int, is_bool);
    let empty = builder.ins().bor(is_null, is_undefined);
    let supported = builder.ins().bor(scalar, empty);
    let supported = builder.ins().bor(supported, is_float);
    guard(builder, supported, retry);
    let integer_truthy = builder.ins().icmp_imm(IntCC::NotEqual, value.payload, 0);
    let float = builder
        .ins()
        .bitcast(types::F64, MemFlags::new(), value.payload);
    let zero = builder.ins().f64const(0.0);
    let float_truthy = builder.ins().fcmp(FloatCC::OrderedNotEqual, float, zero);
    let scalar_truthy = builder.ins().select(is_float, float_truthy, integer_truthy);
    let false_value = builder.ins().iconst(types::I8, 0);
    builder.ins().select(empty, false_value, scalar_truthy)
}

fn emit_unary(
    builder: &mut FunctionBuilder<'_>,
    value: Pair,
    operation: UnaryOp,
    retry: Block,
) -> Pair {
    match operation {
        UnaryOp::LogicalNot => {
            let truthy = emit_truthy(builder, value, retry);
            let inverse = builder.ins().bxor_imm(truthy, 1);
            pair_from_bool(builder, inverse)
        }
        UnaryOp::BitNot => {
            let int = emit_to_i32(builder, value, retry);
            let int = builder.ins().bnot(int);
            Pair {
                payload: builder.ins().sextend(types::I64, int),
                tag: builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
            }
        }
        UnaryOp::Plus => {
            let _ = emit_numeric(builder, value, retry);
            value
        }
        UnaryOp::Neg | UnaryOp::Increment | UnaryOp::Decrement => {
            let (is_int, number) = emit_numeric(builder, value, retry);
            let int = builder.ins().ireduce(types::I32, value.payload);
            let (int_result, overflow) = match operation {
                UnaryOp::Neg => {
                    let zero = builder.ins().iconst(types::I32, 0);
                    builder.ins().ssub_overflow(zero, int)
                }
                UnaryOp::Increment => {
                    let one = builder.ins().iconst(types::I32, 1);
                    builder.ins().sadd_overflow(int, one)
                }
                UnaryOp::Decrement => {
                    let one = builder.ins().iconst(types::I32, 1);
                    builder.ins().ssub_overflow(int, one)
                }
                _ => unreachable!(),
            };
            let no_overflow = builder.ins().bxor_imm(overflow, 1);
            let mut int_fast = builder.ins().band(is_int, no_overflow);
            if operation == UnaryOp::Neg {
                let nonzero = builder.ins().icmp_imm(IntCC::NotEqual, int, 0);
                int_fast = builder.ins().band(int_fast, nonzero);
            }
            let float_result = match operation {
                UnaryOp::Neg => builder.ins().fneg(number),
                UnaryOp::Increment => {
                    let one = builder.ins().f64const(1.0);
                    builder.ins().fadd(number, one)
                }
                UnaryOp::Decrement => {
                    let one = builder.ins().f64const(1.0);
                    builder.ins().fsub(number, one)
                }
                _ => unreachable!(),
            };
            select_int_or_float(builder, int_fast, int_result, float_result)
        }
    }
}

fn emit_binary(
    builder: &mut FunctionBuilder<'_>,
    left: Pair,
    right: Pair,
    operation: BinaryOp,
    retry: Block,
) -> Pair {
    match operation {
        BinaryOp::BitAnd
        | BinaryOp::BitOr
        | BinaryOp::BitXor
        | BinaryOp::ShiftLeft
        | BinaryOp::ShiftRight
        | BinaryOp::ShiftRightUnsigned => {
            emit_integer_binary(builder, left, right, operation, retry)
        }
        BinaryOp::LessThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThan
        | BinaryOp::GreaterThanOrEqual
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::StrictEqual
        | BinaryOp::StrictNotEqual => emit_comparison(builder, left, right, operation, retry),
        _ => emit_arithmetic(builder, left, right, operation, retry),
    }
}

fn emit_integer_binary(
    builder: &mut FunctionBuilder<'_>,
    left: Pair,
    right: Pair,
    operation: BinaryOp,
    retry: Block,
) -> Pair {
    let left = emit_to_i32(builder, left, retry);
    let right = emit_to_i32(builder, right, retry);
    let shift = builder.ins().band_imm(right, 31);
    let result = match operation {
        BinaryOp::BitAnd => builder.ins().band(left, right),
        BinaryOp::BitOr => builder.ins().bor(left, right),
        BinaryOp::BitXor => builder.ins().bxor(left, right),
        BinaryOp::ShiftLeft => builder.ins().ishl(left, shift),
        BinaryOp::ShiftRight => builder.ins().sshr(left, shift),
        BinaryOp::ShiftRightUnsigned => builder.ins().ushr(left, shift),
        _ => unreachable!(),
    };
    if operation == BinaryOp::ShiftRightUnsigned {
        let fits_i32 = builder
            .ins()
            .icmp_imm(IntCC::SignedGreaterThanOrEqual, result, 0);
        let int_payload = builder.ins().sextend(types::I64, result);
        let uint_float = builder.ins().fcvt_from_uint(types::F64, result);
        let float_payload = builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), uint_float);
        let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
        let float_tag = builder
            .ins()
            .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
        Pair {
            payload: builder.ins().select(fits_i32, int_payload, float_payload),
            tag: builder.ins().select(fits_i32, int_tag, float_tag),
        }
    } else {
        Pair {
            payload: builder.ins().sextend(types::I64, result),
            tag: builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
        }
    }
}

fn emit_arithmetic(
    builder: &mut FunctionBuilder<'_>,
    left: Pair,
    right: Pair,
    operation: BinaryOp,
    retry: Block,
) -> Pair {
    if operation == BinaryOp::Mod {
        let unsupported = builder.ins().iconst(types::I8, 0);
        guard(builder, unsupported, retry);
        return constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
    }
    let (left_int, left_float) = emit_numeric(builder, left, retry);
    let (right_int, right_float) = emit_numeric(builder, right, retry);
    let left_i32 = builder.ins().ireduce(types::I32, left.payload);
    let right_i32 = builder.ins().ireduce(types::I32, right.payload);
    if operation == BinaryOp::Div {
        let result = builder.ins().fdiv(left_float, right_float);
        return pair_from_number(builder, result);
    }
    let (int_result, overflow) = match operation {
        BinaryOp::Add => builder.ins().sadd_overflow(left_i32, right_i32),
        BinaryOp::Sub => builder.ins().ssub_overflow(left_i32, right_i32),
        BinaryOp::Mul => builder.ins().smul_overflow(left_i32, right_i32),
        _ => unreachable!(),
    };
    let both_int = builder.ins().band(left_int, right_int);
    let no_overflow = builder.ins().bxor_imm(overflow, 1);
    let mut int_fast = builder.ins().band(both_int, no_overflow);
    if operation == BinaryOp::Mul {
        let result_zero = builder.ins().icmp_imm(IntCC::Equal, int_result, 0);
        let left_negative = builder.ins().icmp_imm(IntCC::SignedLessThan, left_i32, 0);
        let right_negative = builder.ins().icmp_imm(IntCC::SignedLessThan, right_i32, 0);
        let negative = builder.ins().bxor(left_negative, right_negative);
        let negative_zero = builder.ins().band(result_zero, negative);
        let safe_zero = builder.ins().bxor_imm(negative_zero, 1);
        int_fast = builder.ins().band(int_fast, safe_zero);
    }
    let float_result = match operation {
        BinaryOp::Add => builder.ins().fadd(left_float, right_float),
        BinaryOp::Sub => builder.ins().fsub(left_float, right_float),
        BinaryOp::Mul => builder.ins().fmul(left_float, right_float),
        _ => unreachable!(),
    };
    select_int_or_float(builder, int_fast, int_result, float_result)
}

fn select_int_or_float(
    builder: &mut FunctionBuilder<'_>,
    use_int: Value,
    integer: Value,
    float: Value,
) -> Pair {
    let int_payload = builder.ins().sextend(types::I64, integer);
    let number = pair_from_number(builder, float);
    let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
    Pair {
        payload: builder.ins().select(use_int, int_payload, number.payload),
        tag: builder.ins().select(use_int, int_tag, number.tag),
    }
}

fn pair_from_number(builder: &mut FunctionBuilder<'_>, value: Value) -> Pair {
    let integer = builder.ins().fcvt_to_sint_sat(types::I32, value);
    let roundtrip = builder.ins().fcvt_from_sint(types::F64, integer);
    let exact = builder.ins().fcmp(FloatCC::Equal, value, roundtrip);
    let bits = builder.ins().bitcast(types::I64, MemFlags::new(), value);
    let negative_zero = builder.ins().icmp_imm(IntCC::Equal, bits, i64::MIN);
    let not_negative_zero = builder.ins().bxor_imm(negative_zero, 1);
    let use_int = builder.ins().band(exact, not_negative_zero);
    let int_payload = builder.ins().sextend(types::I64, integer);
    let int_tag = builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT));
    let float_tag = builder
        .ins()
        .iconst(types::I64, i64::from(qjs::JS_TAG_FLOAT64));
    Pair {
        payload: builder.ins().select(use_int, int_payload, bits),
        tag: builder.ins().select(use_int, int_tag, float_tag),
    }
}

fn emit_to_i32(builder: &mut FunctionBuilder<'_>, value: Pair, retry: Block) -> Value {
    let (is_int, number) = emit_numeric(builder, value, retry);
    let direct = builder.ins().ireduce(types::I32, value.payload);
    let modulus = builder.ins().f64const(4_294_967_296.0);
    let quotient = builder.ins().fdiv(number, modulus);
    let quotient = builder.ins().trunc(quotient);
    let multiple = builder.ins().fmul(quotient, modulus);
    let remainder = builder.ins().fsub(number, multiple);
    let converted = builder.ins().fcvt_to_sint_sat(types::I64, remainder);
    let converted = builder.ins().ireduce(types::I32, converted);
    builder.ins().select(is_int, direct, converted)
}

fn emit_comparison(
    builder: &mut FunctionBuilder<'_>,
    left: Pair,
    right: Pair,
    operation: BinaryOp,
    retry: Block,
) -> Pair {
    let (_, left) = emit_numeric(builder, left, retry);
    let (_, right) = emit_numeric(builder, right, retry);
    let condition = match operation {
        BinaryOp::LessThan => builder.ins().fcmp(FloatCC::LessThan, left, right),
        BinaryOp::LessThanOrEqual => builder.ins().fcmp(FloatCC::LessThanOrEqual, left, right),
        BinaryOp::GreaterThan => builder.ins().fcmp(FloatCC::GreaterThan, left, right),
        BinaryOp::GreaterThanOrEqual => {
            builder.ins().fcmp(FloatCC::GreaterThanOrEqual, left, right)
        }
        BinaryOp::Equal | BinaryOp::StrictEqual => builder.ins().fcmp(FloatCC::Equal, left, right),
        BinaryOp::NotEqual | BinaryOp::StrictNotEqual => {
            builder.ins().fcmp(FloatCC::NotEqual, left, right)
        }
        _ => unreachable!(),
    };
    pair_from_bool(builder, condition)
}

fn apply_stack_operation(
    builder: &mut FunctionBuilder<'_>,
    stack: &[PairVars],
    depth: &mut usize,
    operation: StackOp,
) {
    let (take, order): (usize, &[usize]) = match operation {
        StackOp::Nip => (2, &[1]),
        StackOp::Nip1 => (3, &[1, 2]),
        StackOp::Dup => (1, &[0, 0]),
        StackOp::Dup1 => (2, &[0, 0, 1]),
        StackOp::Dup2 => (2, &[0, 1, 0, 1]),
        StackOp::Dup3 => (3, &[0, 1, 2, 0, 1, 2]),
        StackOp::Insert2 => (2, &[1, 0, 1]),
        StackOp::Insert3 => (3, &[2, 0, 1, 2]),
        StackOp::Insert4 => (4, &[3, 0, 1, 2, 3]),
        StackOp::Perm3 => (3, &[1, 0, 2]),
        StackOp::Perm4 => (4, &[2, 0, 1, 3]),
        StackOp::Perm5 => (5, &[3, 0, 1, 2, 4]),
        StackOp::Swap => (2, &[1, 0]),
        StackOp::Swap2 => (4, &[2, 3, 0, 1]),
        StackOp::Rot3Left => (3, &[1, 2, 0]),
        StackOp::Rot3Right => (3, &[2, 0, 1]),
        StackOp::Rot4Left => (4, &[1, 2, 3, 0]),
        StackOp::Rot5Left => (5, &[1, 2, 3, 4, 0]),
    };
    let start = *depth - take;
    let values: Vec<Pair> = (0..take)
        .map(|index| use_pair(builder, stack[start + index]))
        .collect();
    for (destination, &source) in order.iter().enumerate() {
        define_pair(builder, stack[start + destination], values[source]);
    }
    *depth = start + order.len();
}
