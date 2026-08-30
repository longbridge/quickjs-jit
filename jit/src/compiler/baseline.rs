//! Cranelift lowering and W^X publication for Tier 1 pure frame operations.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    mem,
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
        SourceLoc, TrapCode, Value,
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
        CodeAllocation, CompiledArtifact, FrameState as ArtifactFrameState, FrameStateLocationKind,
        Relocation, RelocationKind, RelocationTarget, StackMap, UnwindKind, UnwindMetadata,
    },
    ir::{
        BaselineIr, BinaryOp, FrameSlot, FrameStateId, FrameStateKind, IrOp, PollKind, StackOp,
        TaggedValue, UnaryOp, MAX_HELPER_SCRATCH_SLOTS,
    },
    platform::{CodeAllocator, CodeMemoryError, ExecutableCode},
    runtime::CompileRequest,
};

use super::{
    helpers::{generated_signatures, FrameLayout},
    CompileControl, CompileFailure, Compiler,
};

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

#[derive(Clone, Copy)]
struct PollLocation {
    bytecode_pc: u32,
    source_location: SourceLoc,
}

const LOOP_POLL_INTERVAL: i64 = 64;

struct HelperLowering<'a> {
    ir: &'a BaselineIr,
    frame: Value,
    sret: Value,
    arg_buf: Value,
    var_buf: Value,
    stack_base: Value,
    arguments: &'a [PairVars],
    locals: &'a [PairVars],
    stack: &'a [PairVars],
    signatures: &'a [cranelift_codegen::ir::SigRef],
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
}

impl HelperLowering<'_> {
    fn invoke(
        &self,
        builder: &mut FunctionBuilder<'_>,
        helper_id: qjs::JSJitHelperId,
        state: FrameStateId,
        live_depth: usize,
        exception_depth: usize,
        arguments: &[u32],
    ) -> Result<(), CompileFailure> {
        invoke_frame_helper(
            builder,
            self.ir,
            self.frame,
            self.sret,
            self.arg_buf,
            self.var_buf,
            self.stack_base,
            self.arguments,
            self.locals,
            self.stack,
            live_depth,
            exception_depth,
            self.signatures,
            helper_id as usize,
            state,
            arguments,
            self.pointer_type,
            self.layout,
        )
    }

    fn set_depth(
        &self,
        builder: &mut FunctionBuilder<'_>,
        depth: usize,
    ) -> Result<(), CompileFailure> {
        set_visible_stack_depth(builder, self.frame, self.stack_base, depth, self.layout)
    }
}

/// Cranelift compiler configured for one explicit target ISA.
#[derive(Clone)]
pub struct BaselineCompiler {
    isa: OwnedTargetIsa,
    host_publishable: bool,
}

/// Stable, complete identity of the Cranelift target used to produce code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetIdentity {
    triple: String,
    shared_flags: Vec<(String, String)>,
    isa_flags: Vec<(String, String)>,
}

impl TargetIdentity {
    pub fn from_isa(isa: &dyn TargetIsa) -> Self {
        let mut shared_flags = isa
            .flags()
            .iter()
            .map(|value| (value.name.to_owned(), value.value_string()))
            .collect::<Vec<_>>();
        let mut isa_flags = isa
            .isa_flags()
            .into_iter()
            .map(|value| (value.name.to_owned(), value.value_string()))
            .collect::<Vec<_>>();
        shared_flags.sort_unstable();
        isa_flags.sort_unstable();
        Self {
            triple: isa.triple().to_string(),
            shared_flags,
            isa_flags,
        }
    }

    pub fn triple(&self) -> &str {
        &self.triple
    }

    pub fn shared_flags(&self) -> &[(String, String)] {
        &self.shared_flags
    }

    pub fn isa_flags(&self) -> &[(String, String)] {
        &self.isa_flags
    }

    fn hash_fields<'a>(fields: impl IntoIterator<Item = &'a str>) -> u64 {
        let mut hash = 0xcbf29ce484222325_u64;
        for field in fields {
            for byte in (field.len() as u64)
                .to_le_bytes()
                .into_iter()
                .chain(field.bytes())
            {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
        hash
    }

    pub fn triple_fingerprint(&self) -> u64 {
        Self::hash_fields([self.triple.as_str()])
    }

    pub fn codegen_fingerprint(&self) -> u64 {
        Self::hash_fields(
            self.shared_flags
                .iter()
                .flat_map(|(name, value)| ["shared", name.as_str(), value.as_str()])
                .chain(
                    self.isa_flags
                        .iter()
                        .flat_map(|(name, value)| ["isa", name.as_str(), value.as_str()]),
                ),
        )
    }
}

#[derive(Clone, Copy)]
enum CompilePolicy {
    AdvertisedOnly,
    #[cfg(feature = "test-support")]
    ImplementedForCompilerTest,
}

#[derive(Clone, Copy)]
enum GuardExit {
    Retry,
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

    pub fn target_identity(&self) -> TargetIdentity {
        TargetIdentity::from_isa(self.isa())
    }

    /// Compiles verified bytecode without allocating executable memory.
    pub fn compile(&self, function: &VerifiedFunction) -> Result<RelocatableCode, CompileFailure> {
        self.compile_with_policy(function, CompilePolicy::AdvertisedOnly, None)
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn compile_implemented_for_test(
        &self,
        function: &VerifiedFunction,
    ) -> Result<RelocatableCode, CompileFailure> {
        self.compile_with_policy(function, CompilePolicy::ImplementedForCompilerTest, None)
    }

    fn compile_with_policy(
        &self,
        function: &VerifiedFunction,
        policy: CompilePolicy,
        control: Option<&CompileControl>,
    ) -> Result<RelocatableCode, CompileFailure> {
        self.compile_with_policy_start(function, policy, control, None, GuardExit::Retry)
    }

    fn compile_with_policy_start(
        &self,
        function: &VerifiedFunction,
        policy: CompilePolicy,
        control: Option<&CompileControl>,
        osr_start: Option<u32>,
        _guard_exit: GuardExit,
    ) -> Result<RelocatableCode, CompileFailure> {
        if let Some(control) = control {
            control.check()?;
        }
        if self.isa.triple().pointer_width().map(|width| width.bits()) != Ok(64)
            || self.isa.triple().endianness() != Ok(Endianness::Little)
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let pointer_type = self.isa.pointer_type();
        let layout = FrameLayout::validated(
            u8::try_from(pointer_type.bytes()).map_err(|_| CompileFailure::InvalidArtifact)?,
        )?;
        let ir = match policy {
            CompilePolicy::AdvertisedOnly => BaselineIr::translate(function)?,
            #[cfg(feature = "test-support")]
            CompilePolicy::ImplementedForCompilerTest => {
                BaselineIr::translate_implemented_for_test(function)?
            }
        };
        if let Some(control) = control {
            let estimated_ir = function.snapshot().owned_bytes().saturating_mul(32);
            control.check_ir_bytes(estimated_ir)?;
        }
        let logical_stack_capacity = u32::from(function.snapshot().stack_size());
        let scratch_slots =
            u32::try_from(MAX_HELPER_SCRATCH_SLOTS).map_err(|_| CompileFailure::InvalidArtifact)?;
        let stack_capacity = logical_stack_capacity
            .checked_add(scratch_slots)
            .ok_or(CompileFailure::ResourceLimit)?;
        let required_scratch = u32::from(ir.max_stack_depth).saturating_sub(logical_stack_capacity);
        if required_scratch > scratch_slots || u32::from(ir.max_stack_depth) > stack_capacity {
            return Err(CompileFailure::ResourceLimit);
        }
        let entry_analysis = analyze_entry_domains(&ir)?;
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
            lower_function(
                &mut builder,
                &ir,
                &*self.isa,
                layout,
                &entry_analysis,
                osr_start,
                GuardExit::Retry,
            )?;
            builder.seal_all_blocks();
            builder.finalize();
        }
        if let Some(control) = control {
            control.check()?;
        }

        let clif_text = clif.display().to_string();
        if let Some(control) = control {
            control.check_ir_bytes(clif_text.len())?;
        }
        let function_parameters = clif.params.clone();
        let mut context = Context::for_function(clif);
        let compiled = context
            .compile(&*self.isa, &mut ControlPlane::default())
            .map_err(|_| CompileFailure::InvalidArtifact)?;
        if let Some(control) = control {
            control.check()?;
        }
        let unwind_info = compiled
            .create_unwind_info(&*self.isa)
            .map_err(|_| CompileFailure::InvalidArtifact)?
            .ok_or(CompileFailure::InvalidArtifact)?;
        let unwind_metadata = Some(retain_unwind_metadata(&unwind_info, compiled.frame_size)?);
        let native_unwind = NativeUnwindPlan::new(unwind_info, &*self.isa)?;
        let bytes = compiled.code_buffer().to_vec();
        let source_ranges = compiled.buffer.get_srclocs_sorted();
        let call_return_offsets: Vec<_> = compiled
            .buffer
            .call_sites()
            .iter()
            .map(|site| site.ret_addr)
            .collect();
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
        let frame_states: Vec<_> = if entry_analysis.retry_before_entry {
            Vec::new()
        } else {
            ir.frame_states
                .iter()
                .enumerate()
                .filter_map(|state| {
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
                        .collect::<Result<Vec<_>, _>>();
                    let slots = match slots {
                        Ok(slots) => slots,
                        Err(error) => return Some(Err(error)),
                    };
                    let state_id =
                        FrameStateId::from_index(state_index).ok_or(CompileFailure::ResourceLimit);
                    let state_id = match state_id {
                        Ok(state_id) => state_id,
                        Err(error) => return Some(Err(error)),
                    };
                    let source_location = match frame_state_source_loc(state_id) {
                        Ok(location) => location.bits(),
                        Err(error) => return Some(Err(error)),
                    };
                    let matching_ranges: Vec<_> = source_ranges
                        .iter()
                        .filter(|range| range.loc.bits() == source_location)
                        .collect();
                    let [source_range] = matching_ranges.as_slice() else {
                        if osr_start.is_some() && matching_ranges.is_empty() {
                            return None;
                        }
                        return Some(Err(CompileFailure::InvalidArtifact));
                    };
                    let (location_kind, code_offset) = match state.kind {
                        FrameStateKind::Poll | FrameStateKind::Helper => {
                            let matching_calls: Vec<_> = call_return_offsets
                                .iter()
                                .copied()
                                .filter(|return_address| {
                                    source_range.start < *return_address
                                        && *return_address <= source_range.end
                                })
                                .collect();
                            let [return_address] = matching_calls.as_slice() else {
                                return Some(Err(CompileFailure::InvalidArtifact));
                            };
                            (FrameStateLocationKind::CallReturn, *return_address)
                        }
                        FrameStateKind::Marker => {
                            if call_return_offsets.iter().any(|return_address| {
                                source_range.start < *return_address
                                    && *return_address <= source_range.end
                            }) {
                                return Some(Err(CompileFailure::InvalidArtifact));
                            }
                            (FrameStateLocationKind::Marker, source_range.start)
                        }
                    };
                    if code_offset as usize >= bytes.len()
                        || !distinct_state_offsets.insert(code_offset)
                    {
                        return Some(Err(CompileFailure::InvalidArtifact));
                    }
                    Some(Ok(ArtifactFrameState::with_location(
                        code_offset,
                        state.pc,
                        slots,
                        location_kind,
                        source_location,
                        source_range.start,
                        source_range.end,
                    )))
                })
                .collect::<Result<_, _>>()?
        };
        let stack_maps = frame_states
            .iter()
            .map(|state| StackMap::new(state.code_offset, state.slots.to_vec()))
            .collect();
        let mut code = RelocatableCode {
            bytes,
            relocations,
            unwind_metadata,
            native_unwind,
            stack_maps,
            frame_states,
            call_return_offsets,
            target: self.isa.triple().clone(),
            host_publishable: self.host_publishable,
            clif: clif_text,
            osr_codes: Vec::new(),
        };
        if osr_start.is_none() && matches!(policy, CompilePolicy::AdvertisedOnly) {
            for point in function.osr_points() {
                let Some(map) =
                    crate::runtime::OsrMap::from_verified(function, point.pc(), point.pc())
                else {
                    continue;
                };
                let child = self.compile_with_policy_start(
                    function,
                    policy,
                    control,
                    Some(point.pc()),
                    GuardExit::Retry,
                )?;
                code.osr_codes.push((map, child));
            }
        }
        Ok(code)
    }
}

#[cfg(test)]
mod target_identity_tests {
    use super::*;

    #[test]
    fn target_identity_is_the_canonical_identity_of_the_compilers_actual_isa() {
        let compiler = BaselineCompiler::host();
        assert_eq!(
            compiler.target_identity(),
            TargetIdentity::from_isa(compiler.isa())
        );
        assert_eq!(
            compiler.target_identity().triple(),
            compiler.isa().triple().to_string()
        );
        assert!(!compiler.target_identity().shared_flags().is_empty());
        assert!(!compiler.target_identity().isa_flags().is_empty());
    }

    #[test]
    fn every_codegen_setting_value_participates_in_target_identity() {
        let compiler = BaselineCompiler::host();
        let identity = compiler.target_identity();
        assert!(!identity.shared_flags().is_empty());
        for index in 0..identity.shared_flags().len() {
            let mut changed = identity.clone();
            changed.shared_flags[index].1.push_str("-changed");
            assert_ne!(
                identity.codegen_fingerprint(),
                changed.codegen_fingerprint(),
                "setting {} was omitted from the identity",
                identity.shared_flags()[index].0
            );
        }
        for index in 0..identity.isa_flags().len() {
            let mut changed = identity.clone();
            changed.isa_flags[index].1.push_str("-changed");
            assert_ne!(
                identity.codegen_fingerprint(),
                changed.codegen_fingerprint()
            );
        }
    }
}

impl Compiler for BaselineCompiler {
    fn compile(&self, request: CompileRequest) -> Result<CompiledArtifact, CompileFailure> {
        let code = BaselineCompiler::compile(self, request.snapshot())?;
        Ok(artifact_from_relocatable(request, code))
    }

    fn compile_controlled(
        &self,
        request: CompileRequest,
        control: &CompileControl,
    ) -> Result<CompiledArtifact, CompileFailure> {
        let code = self.compile_with_policy(
            request.snapshot(),
            CompilePolicy::AdvertisedOnly,
            Some(control),
        )?;
        control.check()?;
        Ok(artifact_from_relocatable(request, code))
    }
}

/// Finalizes Cranelift IR produced by the independent optimizing builder. This
/// owns only target encoding/unwind packaging; it does not translate or lower
/// baseline IR.
pub(crate) fn finalize_optimized_machine(
    isa: &OwnedTargetIsa,
    clif: Function,
    control: Option<&CompileControl>,
) -> Result<RelocatableCode, CompileFailure> {
    if let Some(control) = control {
        control.check()?;
        control.check_ir_bytes(clif.display().to_string().len())?;
    }
    let clif_text = clif.display().to_string();
    let function_parameters = clif.params.clone();
    let mut context = Context::for_function(clif);
    let compiled = context
        .compile(&**isa, &mut ControlPlane::default())
        .map_err(|_| CompileFailure::InvalidArtifact)?;
    if let Some(control) = control {
        control.check()?;
    }
    let unwind_info = compiled
        .create_unwind_info(&**isa)
        .map_err(|_| CompileFailure::InvalidArtifact)?
        .ok_or(CompileFailure::InvalidArtifact)?;
    let unwind_metadata = Some(retain_unwind_metadata(&unwind_info, compiled.frame_size)?);
    let native_unwind = NativeUnwindPlan::new(unwind_info, &**isa)?;
    let bytes = compiled.code_buffer().to_vec();
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
    Ok(RelocatableCode {
        bytes,
        relocations,
        unwind_metadata,
        native_unwind,
        stack_maps: Vec::new(),
        frame_states: Vec::new(),
        call_return_offsets: compiled
            .buffer
            .call_sites()
            .iter()
            .map(|site| site.ret_addr)
            .collect(),
        target: isa.triple().clone(),
        host_publishable: isa_is_host_compatible(&**isa),
        clif: clif_text,
        osr_codes: Vec::new(),
    })
}

pub(crate) fn artifact_from_relocatable(
    request: CompileRequest,
    code: RelocatableCode,
) -> CompiledArtifact {
    let mut charged_code = Vec::with_capacity(code.total_code_bytes());
    let mut charged_relocations = Vec::new();
    let mut charged_stack_maps = Vec::new();
    let mut charged_frame_states = Vec::new();
    code.collect_charge_metadata(
        &mut charged_code,
        &mut charged_relocations,
        &mut charged_stack_maps,
        &mut charged_frame_states,
    );
    CompiledArtifact::from_parts(
        request.artifact_key(),
        CodeAllocation::inert(charged_code),
        charged_relocations,
        charged_stack_maps,
        charged_frame_states,
        Vec::new(),
    )
    .with_unwind_metadata(code.unwind_metadata.clone())
    .with_relocatable(code)
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
    call_return_offsets: Vec<u32>,
    target: Triple,
    host_publishable: bool,
    clif: String,
    osr_codes: Vec<(crate::runtime::OsrMap, RelocatableCode)>,
}

impl RelocatableCode {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn osr_entry_count(&self) -> usize {
        self.osr_codes.len()
    }

    pub fn total_code_bytes(&self) -> usize {
        self.osr_codes
            .iter()
            .fold(self.bytes.len(), |total, (_, code)| {
                total.saturating_add(code.total_code_bytes())
            })
    }

    fn collect_charge_metadata(
        &self,
        code_bytes: &mut Vec<u8>,
        relocations: &mut Vec<Relocation>,
        stack_maps: &mut Vec<StackMap>,
        frame_states: &mut Vec<ArtifactFrameState>,
    ) {
        code_bytes.extend_from_slice(&self.bytes);
        relocations.extend_from_slice(&self.relocations);
        stack_maps.extend_from_slice(&self.stack_maps);
        frame_states.extend_from_slice(&self.frame_states);
        for (_, code) in &self.osr_codes {
            code.collect_charge_metadata(code_bytes, relocations, stack_maps, frame_states);
        }
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

    pub fn call_return_offsets(&self) -> &[u32] {
        &self.call_return_offsets
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
        let osr_codes = self
            .osr_codes
            .into_iter()
            .map(|(map, code)| Ok((map, code.publish()?)))
            .collect::<Result<Vec<_>, CodeMemoryError>>()?;
        Ok(PublishedBaselineCode::new(
            executable,
            unwind_registration,
            self.unwind_metadata,
            self.stack_maps,
            self.frame_states,
            osr_codes,
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
    osr_codes: Box<[(crate::runtime::OsrMap, PublishedBaselineCode)]>,
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
        osr_codes: Vec<(crate::runtime::OsrMap, PublishedBaselineCode)>,
    ) -> Self {
        Self {
            allocation: Arc::new(PublishedBaselineAllocation {
                unwind_registration: Some(unwind_registration),
                executable: Some(executable),
                unwind_metadata,
                stack_maps: stack_maps.into_boxed_slice(),
                frame_states: frame_states.into_boxed_slice(),
                osr_codes: osr_codes.into_boxed_slice(),
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

    pub fn required_stack_map_count(&self) -> u32 {
        self.allocation
            .frame_states
            .iter()
            .map(|state| state.source_location)
            .max()
            .and_then(|maximum| maximum.checked_add(1))
            .unwrap_or(0)
    }

    pub fn frame_states(&self) -> &[ArtifactFrameState] {
        &self.allocation.frame_states
    }

    pub fn osr_entry(&self, pc: u32) -> Option<(&crate::runtime::OsrMap, &PublishedBaselineCode)> {
        self.allocation
            .osr_codes
            .iter()
            .find(|(map, _)| map.key().pc() == pc)
            .map(|(map, code)| (map, code))
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
        .filter(|bits| *bits != u32::MAX)
        .ok_or(CompileFailure::ResourceLimit)?;
    Ok(SourceLoc::new(bits))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum EntryRoot {
    Argument(u16),
    Local(u16),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum KnownKind {
    Number,
    Boolean,
    Null,
    Undefined,
    ShortBigInt,
    Uninitialized,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequiredDomain {
    Numeric,
    Initialized,
    Uninitialized,
}

impl KnownKind {
    fn satisfies(self, required: RequiredDomain) -> bool {
        match required {
            RequiredDomain::Numeric => self == Self::Number,
            RequiredDomain::Initialized => self != Self::Uninitialized,
            RequiredDomain::Uninitialized => self == Self::Uninitialized,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AbstractValue {
    roots: BTreeSet<EntryRoot>,
    known: BTreeSet<KnownKind>,
}

impl AbstractValue {
    fn root(root: EntryRoot) -> Self {
        Self {
            roots: BTreeSet::from([root]),
            known: BTreeSet::new(),
        }
    }

    fn known(kind: KnownKind) -> Self {
        Self {
            roots: BTreeSet::new(),
            known: BTreeSet::from([kind]),
        }
    }

    fn from_tagged(value: TaggedValue) -> Self {
        let tag = value.tag;
        let kind = if tag == i64::from(qjs::JS_TAG_INT) || tag == i64::from(qjs::JS_TAG_FLOAT64) {
            KnownKind::Number
        } else if tag == i64::from(qjs::JS_TAG_BOOL) {
            KnownKind::Boolean
        } else if tag == i64::from(qjs::JS_TAG_NULL) {
            KnownKind::Null
        } else if tag == i64::from(qjs::JS_TAG_UNDEFINED) {
            KnownKind::Undefined
        } else if tag == i64::from(qjs::JS_TAG_SHORT_BIG_INT) {
            KnownKind::ShortBigInt
        } else if tag == i64::from(qjs::JS_TAG_UNINITIALIZED) {
            KnownKind::Uninitialized
        } else {
            KnownKind::Other
        };
        Self::known(kind)
    }

    fn merge_from(&mut self, other: &Self) -> bool {
        let old_roots = self.roots.len();
        let old_known = self.known.len();
        self.roots.extend(other.roots.iter().copied());
        self.known.extend(other.known.iter().copied());
        old_roots != self.roots.len() || old_known != self.known.len()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RootRequirements {
    numeric: bool,
    initialized: bool,
    uninitialized: bool,
}

impl RootRequirements {
    fn add(&mut self, required: RequiredDomain) -> bool {
        match required {
            RequiredDomain::Numeric => {
                self.numeric = true;
                self.initialized = true;
            }
            RequiredDomain::Initialized => self.initialized = true,
            RequiredDomain::Uninitialized => self.uninitialized = true,
        }
        self.uninitialized && (self.numeric || self.initialized)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AbstractFrame {
    arguments: Vec<AbstractValue>,
    locals: Vec<AbstractValue>,
    stack: Vec<AbstractValue>,
}

impl AbstractFrame {
    fn merge_from(&mut self, other: &Self) -> Result<bool, CompileFailure> {
        if self.arguments.len() != other.arguments.len()
            || self.locals.len() != other.locals.len()
            || self.stack.len() != other.stack.len()
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let mut changed = false;
        for (current, incoming) in self
            .arguments
            .iter_mut()
            .zip(&other.arguments)
            .chain(self.locals.iter_mut().zip(&other.locals))
            .chain(self.stack.iter_mut().zip(&other.stack))
        {
            changed |= current.merge_from(incoming);
        }
        Ok(changed)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct EntryAnalysis {
    retry_before_entry: bool,
    requirements: BTreeMap<EntryRoot, RootRequirements>,
}

impl EntryAnalysis {
    fn require(&mut self, value: &AbstractValue, required: RequiredDomain) -> bool {
        if value
            .known
            .iter()
            .copied()
            .any(|kind| !kind.satisfies(required))
        {
            self.retry_before_entry = true;
            return false;
        }
        for root in &value.roots {
            if self.requirements.entry(*root).or_default().add(required) {
                self.retry_before_entry = true;
                return false;
            }
        }
        true
    }
}

fn analyze_entry_domains(ir: &BaselineIr) -> Result<EntryAnalysis, CompileFailure> {
    let block_indices: BTreeMap<_, _> = ir
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.start_pc, index))
        .collect();
    let Some(&entry_index) = block_indices.get(&0) else {
        return Err(CompileFailure::InvalidArtifact);
    };
    let entry = AbstractFrame {
        arguments: (0..ir.argument_count)
            .map(|index| AbstractValue::root(EntryRoot::Argument(index)))
            .collect(),
        locals: (0..ir.local_count)
            .map(|index| AbstractValue::root(EntryRoot::Local(index)))
            .collect(),
        stack: Vec::new(),
    };
    let mut block_inputs = vec![None; ir.blocks.len()];
    block_inputs[entry_index] = Some(entry);
    let mut queue = VecDeque::from([entry_index]);
    let mut queued = BTreeSet::from([entry_index]);
    let mut analysis = EntryAnalysis::default();

    while let Some(block_index) = queue.pop_front() {
        queued.remove(&block_index);
        let block = &ir.blocks[block_index];
        let mut frame = block_inputs[block_index]
            .clone()
            .ok_or(CompileFailure::InvalidArtifact)?;
        if frame.stack.len() != usize::from(block.stack_depth) {
            return Err(CompileFailure::InvalidArtifact);
        }
        let mut successors = Vec::new();
        let mut terminated = false;

        for instruction in &block.instructions {
            match &instruction.op {
                IrOp::Poll { .. } | IrOp::OsrLabel { state: _ } | IrOp::Nop => {}
                IrOp::Push(value) => frame.stack.push(AbstractValue::from_tagged(*value)),
                IrOp::ResolveConstant(_) | IrOp::NewObject => {
                    frame.stack.push(AbstractValue::known(KnownKind::Other));
                }
                IrOp::NewArrayFrom(count) => {
                    let new_len = frame
                        .stack
                        .len()
                        .checked_sub(usize::from(*count))
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    frame.stack.truncate(new_len);
                    frame.stack.push(AbstractValue::known(KnownKind::Other));
                }
                IrOp::GetProperty(_) => {
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    frame.stack.push(AbstractValue::known(KnownKind::Other));
                }
                IrOp::SetProperty(_) => {
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                }
                IrOp::Call { argc, has_this } => {
                    let pop = usize::from(*argc) + 1 + usize::from(*has_this);
                    let new_len = frame
                        .stack
                        .len()
                        .checked_sub(pop)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    frame.stack.truncate(new_len);
                    frame.stack.push(AbstractValue::known(KnownKind::Other));
                }
                IrOp::GetArgument(index) => frame.stack.push(
                    frame
                        .arguments
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?,
                ),
                IrOp::GetLocal(index) => frame.stack.push(
                    frame
                        .locals
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?,
                ),
                IrOp::GetLocalChecked(index) => {
                    let value = frame
                        .locals
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    if !analysis.require(&value, RequiredDomain::Initialized) {
                        return Ok(analysis);
                    }
                    frame.stack.push(value);
                }
                IrOp::GetLocalPair => {
                    frame.stack.push(
                        frame
                            .locals
                            .first()
                            .cloned()
                            .ok_or(CompileFailure::InvalidArtifact)?,
                    );
                    frame.stack.push(
                        frame
                            .locals
                            .get(1)
                            .cloned()
                            .ok_or(CompileFailure::InvalidArtifact)?,
                    );
                }
                IrOp::PutArgument { index, keep } => {
                    let source = frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    *frame
                        .arguments
                        .get_mut(usize::from(*index))
                        .ok_or(CompileFailure::InvalidArtifact)? = source;
                    if !keep {
                        frame.stack.pop();
                    }
                }
                IrOp::PutLocal { index, keep } => {
                    let source = frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    *frame
                        .locals
                        .get_mut(usize::from(*index))
                        .ok_or(CompileFailure::InvalidArtifact)? = source;
                    if !keep {
                        frame.stack.pop();
                    }
                }
                IrOp::PutLocalChecked { index, initialize } => {
                    let current = frame
                        .locals
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let required = if *initialize {
                        RequiredDomain::Uninitialized
                    } else {
                        RequiredDomain::Initialized
                    };
                    if !analysis.require(&current, required) {
                        return Ok(analysis);
                    }
                    let source = frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    frame.locals[usize::from(*index)] = source;
                }
                IrOp::SetLocalUninitialized(index) => {
                    *frame
                        .locals
                        .get_mut(usize::from(*index))
                        .ok_or(CompileFailure::InvalidArtifact)? =
                        AbstractValue::known(KnownKind::Uninitialized);
                }
                IrOp::Drop => {
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                }
                IrOp::Stack(operation) => {
                    apply_abstract_stack_operation(&mut frame.stack, *operation)?;
                }
                IrOp::Unary(operation) => {
                    let value = frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let result = analyze_unary(&mut analysis, value, *operation)?;
                    if analysis.retry_before_entry {
                        return Ok(analysis);
                    }
                    frame.stack.push(result);
                }
                IrOp::PostUnary(operation) => {
                    let value = frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let result = analyze_unary(&mut analysis, value, *operation)?;
                    if analysis.retry_before_entry {
                        return Ok(analysis);
                    }
                    frame.stack.push(result);
                }
                IrOp::LocalUnary { index, op } => {
                    let value = frame
                        .locals
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let result = analyze_unary(&mut analysis, value, *op)?;
                    if analysis.retry_before_entry {
                        return Ok(analysis);
                    }
                    frame.locals[usize::from(*index)] = result;
                }
                IrOp::AddLocal(index) => {
                    let right = frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let left = frame
                        .locals
                        .get(usize::from(*index))
                        .cloned()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    if !analysis.require(&left, RequiredDomain::Numeric)
                        || !analysis.require(&right, RequiredDomain::Numeric)
                    {
                        return Ok(analysis);
                    }
                    frame.locals[usize::from(*index)] = AbstractValue::known(KnownKind::Number);
                }
                IrOp::Binary(operation) => {
                    if *operation == BinaryOp::Mod {
                        analysis.retry_before_entry = true;
                        return Ok(analysis);
                    }
                    let right = frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let left = frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let helper = matches!(
                        operation,
                        BinaryOp::Add
                            | BinaryOp::LessThan
                            | BinaryOp::LessThanOrEqual
                            | BinaryOp::GreaterThan
                            | BinaryOp::GreaterThanOrEqual
                            | BinaryOp::Equal
                            | BinaryOp::NotEqual
                            | BinaryOp::StrictEqual
                            | BinaryOp::StrictNotEqual
                    );
                    if !helper
                        && (!analysis.require(&left, RequiredDomain::Numeric)
                            || !analysis.require(&right, RequiredDomain::Numeric))
                    {
                        return Ok(analysis);
                    }
                    frame.stack.push(AbstractValue::known(
                        if binary_returns_boolean(*operation) {
                            KnownKind::Boolean
                        } else {
                            KnownKind::Number
                        },
                    ));
                }
                IrOp::Jump(target) => {
                    successors.push((*target, frame.clone()));
                    terminated = true;
                }
                IrOp::Branch {
                    target,
                    when_true: _,
                } => {
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    let fallthrough = ir
                        .blocks
                        .get(block_index + 1)
                        .ok_or(CompileFailure::InvalidArtifact)?
                        .start_pc;
                    successors.push((*target, frame.clone()));
                    successors.push((fallthrough, frame.clone()));
                    terminated = true;
                }
                IrOp::Return => {
                    frame.stack.pop().ok_or(CompileFailure::InvalidArtifact)?;
                    terminated = true;
                }
                IrOp::ReturnUndefined => terminated = true,
            }
            if terminated {
                break;
            }
        }

        if !terminated {
            let next = ir
                .blocks
                .get(block_index + 1)
                .ok_or(CompileFailure::InvalidArtifact)?;
            successors.push((next.start_pc, frame));
        }

        for (successor_pc, incoming) in successors {
            let successor_index = *block_indices
                .get(&successor_pc)
                .ok_or(CompileFailure::InvalidArtifact)?;
            if incoming.stack.len() != usize::from(ir.blocks[successor_index].stack_depth) {
                return Err(CompileFailure::InvalidArtifact);
            }
            let changed = if let Some(existing) = &mut block_inputs[successor_index] {
                existing.merge_from(&incoming)?
            } else {
                block_inputs[successor_index] = Some(incoming);
                true
            };
            if changed && queued.insert(successor_index) {
                queue.push_back(successor_index);
            }
        }
    }

    Ok(analysis)
}

fn analyze_unary(
    analysis: &mut EntryAnalysis,
    value: AbstractValue,
    operation: UnaryOp,
) -> Result<AbstractValue, CompileFailure> {
    let (required, result) = match operation {
        UnaryOp::LogicalNot => return Ok(AbstractValue::known(KnownKind::Boolean)),
        UnaryOp::Plus => return Ok(AbstractValue::known(KnownKind::Other)),
        UnaryOp::Neg | UnaryOp::Increment | UnaryOp::Decrement | UnaryOp::BitNot => {
            (RequiredDomain::Numeric, KnownKind::Number)
        }
    };
    let _ = analysis.require(&value, required);
    Ok(AbstractValue::known(result))
}

fn binary_returns_boolean(operation: BinaryOp) -> bool {
    matches!(
        operation,
        BinaryOp::LessThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThan
            | BinaryOp::GreaterThanOrEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::StrictEqual
            | BinaryOp::StrictNotEqual
    )
}

fn apply_abstract_stack_operation(
    stack: &mut Vec<AbstractValue>,
    operation: StackOp,
) -> Result<(), CompileFailure> {
    let (take, order): (usize, &[usize]) = stack_operation_order(operation);
    let start = stack
        .len()
        .checked_sub(take)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let values = stack[start..].to_vec();
    stack.truncate(start);
    stack.extend(order.iter().map(|source| values[*source].clone()));
    Ok(())
}

fn lower_function(
    builder: &mut FunctionBuilder<'_>,
    ir: &BaselineIr,
    isa: &dyn TargetIsa,
    layout: FrameLayout,
    analysis: &EntryAnalysis,
    osr_start: Option<u32>,
    _guard_exit: GuardExit,
) -> Result<(), CompileFailure> {
    let pointer_type = isa.pointer_type();
    let blocks: BTreeMap<u32, Block> = ir
        .blocks
        .iter()
        .map(|block| (block.start_pc, builder.create_block()))
        .collect();
    let retry = builder.create_block();
    let prologue = builder.create_block();
    let osr_post_poll = osr_start.map(|_| builder.create_block());
    let entry_pc = osr_start.unwrap_or(0);
    let entry = *blocks
        .get(&entry_pc)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let entry_depth = ir
        .blocks
        .iter()
        .find(|block| block.start_pc == entry_pc)
        .map(|block| usize::from(block.stack_depth))
        .ok_or(CompileFailure::InvalidArtifact)?;
    builder.append_block_params_for_function_params(prologue);
    builder.switch_to_block(prologue);
    let params = builder.block_params(prologue);
    if params.len() != 2 {
        return Err(CompileFailure::InvalidArtifact);
    }
    let sret = params[0];
    let frame = params[1];

    if analysis.retry_before_entry {
        emit_exit(
            builder,
            sret,
            qjs::JSJitExitKind_JS_JIT_EXIT_RETRY_INTERPRETER,
            None,
            pointer_type,
        );
        return Ok(());
    }
    let invariant_trap = builder.create_block();

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
    let loop_poll_budget = Variable::from_u32(next_variable);
    for pair in arguments.iter().chain(&locals).chain(&stack) {
        builder.declare_var(pair.payload, types::I64);
        builder.declare_var(pair.tag, types::I64);
    }
    builder.declare_var(loop_poll_budget, types::I32);
    let initial_loop_poll_budget = builder.ins().iconst(types::I32, LOOP_POLL_INTERVAL);
    builder.def_var(loop_poll_budget, initial_loop_poll_budget);
    let flags = MemFlags::new();
    let arg_buf = builder
        .ins()
        .load(pointer_type, flags, frame, layout.arg_buf);
    let var_buf = builder
        .ins()
        .load(pointer_type, flags, frame, layout.var_buf);
    let stack_base = builder
        .ins()
        .load(pointer_type, flags, frame, layout.stack_base);
    for (index, pair) in arguments.iter().copied().enumerate() {
        let value = load_jsvalue(builder, arg_buf, index, layout);
        define_pair(builder, pair, value);
    }
    for (index, pair) in locals.iter().copied().enumerate() {
        let value = load_jsvalue(builder, var_buf, index, layout);
        define_pair(builder, pair, value);
    }
    emit_entry_domain_guards(builder, &arguments, &locals, analysis, retry);
    validate_live_stack_bounds(
        builder,
        frame,
        retry,
        pointer_type,
        layout,
        ir.max_stack_depth,
    );
    for (index, pair) in stack.iter().copied().enumerate() {
        let value = if osr_start.is_some() && index < entry_depth {
            load_jsvalue(builder, stack_base, index, layout)
        } else {
            constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64))
        };
        define_pair(builder, pair, value);
    }

    let helper_signatures = generated_signatures(isa)?
        .into_iter()
        .map(|signature| builder.import_signature(signature))
        .collect::<Vec<_>>();
    let helper_lowering = HelperLowering {
        ir,
        frame,
        sret,
        arg_buf,
        var_buf,
        stack_base,
        arguments: &arguments,
        locals: &locals,
        stack: &stack,
        signatures: &helper_signatures,
        pointer_type,
        layout,
    };

    macro_rules! invoke_helper {
        ($helper_id:expr, $state:expr, $live_depth:expr, $arguments:expr) => {{
            helper_lowering.invoke(
                builder,
                $helper_id,
                $state,
                $live_depth,
                $live_depth,
                $arguments,
            )?;
        }};
    }

    builder.ins().jump(osr_post_poll.unwrap_or(entry), &[]);
    builder.switch_to_block(entry);

    for (block_index, block) in ir.blocks.iter().enumerate() {
        let clif_block = blocks[&block.start_pc];
        if clif_block != entry || osr_start.is_some() {
            builder.switch_to_block(clif_block);
        }
        let mut depth = block.stack_depth as usize;
        let mut terminated = false;
        let mut entered_osr_continuation = false;
        for instruction in &block.instructions {
            let mut helper_states = instruction.helper_states.iter().copied();
            builder.set_srcloc(SourceLoc::default());
            if !matches!(&instruction.op, IrOp::Poll { .. }) {
                if let Some(state) = instruction.frame_state {
                    emit_frame_state_marker(builder, frame, invariant_trap, state)?;
                }
            }
            match instruction.op {
                IrOp::Poll { state, kind } => {
                    let loop_continuation = if kind == PollKind::LoopHeader {
                        let remaining = builder.use_var(loop_poll_budget);
                        let remaining = builder.ins().iadd_imm(remaining, -1);
                        builder.def_var(loop_poll_budget, remaining);
                        let due = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
                        let slow = builder.create_block();
                        let continuation = builder.create_block();
                        builder.ins().brif(due, slow, &[], continuation, &[]);
                        builder.seal_block(slow);
                        builder.switch_to_block(slow);
                        let reset = builder.ins().iconst(types::I32, LOOP_POLL_INTERVAL);
                        builder.def_var(loop_poll_budget, reset);
                        Some(continuation)
                    } else {
                        None
                    };
                    materialize_frame(
                        builder,
                        frame,
                        arg_buf,
                        var_buf,
                        stack_base,
                        &arguments,
                        &locals,
                        &stack,
                        depth,
                        depth,
                        ir.frame_states.get(state).pc,
                        pointer_type,
                        layout,
                    )?;
                    emit_poll(
                        builder,
                        frame,
                        sret,
                        helper_signatures[qjs::JSJitHelperId_JS_JIT_HELPER_POLL as usize],
                        PollLocation {
                            bytecode_pc: instruction.pc,
                            source_location: frame_state_source_loc(state)?,
                        },
                        pointer_type,
                        layout,
                    );
                    if let Some(continuation) = loop_continuation {
                        builder.ins().jump(continuation, &[]);
                        builder.seal_block(continuation);
                        builder.switch_to_block(continuation);
                    }
                    if osr_start == Some(block.start_pc) && !entered_osr_continuation {
                        let continuation = osr_post_poll.ok_or(CompileFailure::InvalidArtifact)?;
                        builder.ins().jump(continuation, &[]);
                        builder.switch_to_block(continuation);
                        entered_osr_continuation = true;
                    }
                }
                IrOp::OsrLabel { .. } => {}
                IrOp::Nop => {}
                IrOp::Push(value) => {
                    let value = constant_pair(builder, value);
                    define_pair(builder, stack[depth], value);
                    depth += 1;
                }
                IrOp::ResolveConstant(index) => {
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let output = flat_stack_slot(ir, depth)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_RESOLVE_CONST,
                        state,
                        depth,
                        &[output, index]
                    );
                    reload_pair(builder, stack[depth], stack_base, depth, layout);
                    depth += 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::NewObject => {
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let output = flat_stack_slot(ir, depth)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_NEW_OBJECT,
                        state,
                        depth,
                        &[output]
                    );
                    reload_pair(builder, stack[depth], stack_base, depth, layout);
                    depth += 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::NewArrayFrom(count) => lower_new_array(
                    builder,
                    &helper_lowering,
                    &mut helper_states,
                    &mut depth,
                    count,
                )?,
                IrOp::GetProperty(atom) => {
                    lower_get_property(builder, &helper_lowering, &mut helper_states, depth, atom)?
                }
                IrOp::SetProperty(atom) => lower_set_property(
                    builder,
                    &helper_lowering,
                    &mut helper_states,
                    &mut depth,
                    atom,
                )?,
                IrOp::Call { argc, has_this } => lower_call(
                    builder,
                    &helper_lowering,
                    &mut helper_states,
                    &mut depth,
                    argc,
                    has_this,
                )?,
                IrOp::GetArgument(index) => {
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let output = flat_stack_slot(ir, depth)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                        state,
                        depth,
                        &[output, flat_argument_slot(index)]
                    );
                    reload_pair(builder, stack[depth], stack_base, depth, layout);
                    depth += 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::GetLocal(index) => {
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let output = flat_stack_slot(ir, depth)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                        state,
                        depth,
                        &[output, flat_local_slot(ir, index)]
                    );
                    reload_pair(builder, stack[depth], stack_base, depth, layout);
                    depth += 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::GetLocalChecked(index) => {
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let output = flat_stack_slot(ir, depth)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                        state,
                        depth,
                        &[output, flat_local_slot(ir, index)]
                    );
                    reload_pair(builder, stack[depth], stack_base, depth, layout);
                    depth += 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::GetLocalPair => {
                    for local_index in 0..2_u16 {
                        let state = helper_states
                            .next()
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        let output_index = depth + usize::from(local_index);
                        let output = flat_stack_slot(ir, output_index)?;
                        invoke_helper!(
                            qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                            state,
                            output_index,
                            &[output, flat_local_slot(ir, local_index)]
                        );
                        reload_pair(
                            builder,
                            stack[output_index],
                            stack_base,
                            output_index,
                            layout,
                        );
                    }
                    depth += 2;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::PutArgument { index, keep } => {
                    let source_index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let destination = flat_argument_slot(index);
                    let free_state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                        free_state,
                        depth,
                        &[destination]
                    );
                    reload_pair(
                        builder,
                        arguments[index as usize],
                        arg_buf,
                        index as usize,
                        layout,
                    );
                    if keep {
                        let dup_state = helper_states
                            .next()
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        invoke_helper!(
                            qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                            dup_state,
                            depth,
                            &[destination, flat_stack_slot(ir, source_index)?]
                        );
                        reload_pair(
                            builder,
                            arguments[index as usize],
                            arg_buf,
                            index as usize,
                            layout,
                        );
                    } else {
                        let value = use_pair(builder, stack[source_index]);
                        define_pair(builder, arguments[index as usize], value);
                        store_jsvalue_slot(builder, arg_buf, index as usize, value, layout)?;
                        clear_pair(
                            builder,
                            stack[source_index],
                            stack_base,
                            source_index,
                            layout,
                        )?;
                        depth -= 1;
                        set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                    }
                }
                IrOp::PutLocal { index, keep } => {
                    let source_index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let destination = flat_local_slot(ir, index);
                    let free_state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                        free_state,
                        depth,
                        &[destination]
                    );
                    reload_pair(
                        builder,
                        locals[index as usize],
                        var_buf,
                        index as usize,
                        layout,
                    );
                    if keep {
                        let dup_state = helper_states
                            .next()
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        invoke_helper!(
                            qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                            dup_state,
                            depth,
                            &[destination, flat_stack_slot(ir, source_index)?]
                        );
                        reload_pair(
                            builder,
                            locals[index as usize],
                            var_buf,
                            index as usize,
                            layout,
                        );
                    } else {
                        let value = use_pair(builder, stack[source_index]);
                        define_pair(builder, locals[index as usize], value);
                        store_jsvalue_slot(builder, var_buf, index as usize, value, layout)?;
                        clear_pair(
                            builder,
                            stack[source_index],
                            stack_base,
                            source_index,
                            layout,
                        )?;
                        depth -= 1;
                        set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                    }
                }
                IrOp::PutLocalChecked { index, initialize } => {
                    let _ = initialize;
                    let source_index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let destination = flat_local_slot(ir, index);
                    let free_state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                        free_state,
                        depth,
                        &[destination]
                    );
                    reload_pair(
                        builder,
                        locals[index as usize],
                        var_buf,
                        index as usize,
                        layout,
                    );
                    let value = use_pair(builder, stack[source_index]);
                    define_pair(builder, locals[index as usize], value);
                    store_jsvalue_slot(builder, var_buf, index as usize, value, layout)?;
                    clear_pair(
                        builder,
                        stack[source_index],
                        stack_base,
                        source_index,
                        layout,
                    )?;
                    depth -= 1;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::SetLocalUninitialized(index) => {
                    let destination = flat_local_slot(ir, index);
                    let free_state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                        free_state,
                        depth,
                        &[destination]
                    );
                    let value = constant_pair(
                        builder,
                        TaggedValue::new(0, qjs::JS_TAG_UNINITIALIZED as i64),
                    );
                    define_pair(builder, locals[index as usize], value);
                    store_jsvalue_slot(builder, var_buf, index as usize, value, layout)?;
                }
                IrOp::Drop => {
                    let index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                        state,
                        depth,
                        &[flat_stack_slot(ir, index)?]
                    );
                    reload_pair(builder, stack[index], stack_base, index, layout);
                    depth = index;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::Stack(operation) => {
                    let old_depth = depth;
                    match operation {
                        StackOp::Nip | StackOp::Nip1 => {
                            let (take, _) = stack_operation_order(operation);
                            let start = depth
                                .checked_sub(take)
                                .ok_or(CompileFailure::InvalidArtifact)?;
                            let state = helper_states
                                .next()
                                .ok_or(CompileFailure::InvalidArtifact)?;
                            invoke_helper!(
                                qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
                                state,
                                depth,
                                &[flat_stack_slot(ir, start)?]
                            );
                            for (offset, variables) in stack
                                .get(start..depth)
                                .ok_or(CompileFailure::InvalidArtifact)?
                                .iter()
                                .copied()
                                .enumerate()
                            {
                                let index = start + offset;
                                reload_pair(builder, variables, stack_base, index, layout);
                            }
                            apply_stack_operation(builder, &stack, &mut depth, operation);
                        }
                        StackOp::Dup
                        | StackOp::Dup1
                        | StackOp::Dup2
                        | StackOp::Dup3
                        | StackOp::Insert2
                        | StackOp::Insert3
                        | StackOp::Insert4 => {
                            let (take, order) = stack_operation_order(operation);
                            let start = depth
                                .checked_sub(take)
                                .ok_or(CompileFailure::InvalidArtifact)?;
                            let duplicated_sources: &[usize] = match operation {
                                StackOp::Dup | StackOp::Dup1 => &[0],
                                StackOp::Dup2 => &[0, 1],
                                StackOp::Dup3 => &[0, 1, 2],
                                StackOp::Insert2 => &[1],
                                StackOp::Insert3 => &[2],
                                StackOp::Insert4 => &[3],
                                _ => unreachable!(),
                            };
                            for (created, source) in duplicated_sources.iter().copied().enumerate()
                            {
                                let state = helper_states
                                    .next()
                                    .ok_or(CompileFailure::InvalidArtifact)?;
                                let output_index = old_depth + created;
                                invoke_helper!(
                                    qjs::JSJitHelperId_JS_JIT_HELPER_DUP,
                                    state,
                                    output_index,
                                    &[
                                        flat_stack_slot(ir, output_index)?,
                                        flat_stack_slot(ir, start + source)?,
                                    ]
                                );
                                reload_pair(
                                    builder,
                                    stack[output_index],
                                    stack_base,
                                    output_index,
                                    layout,
                                );
                            }
                            debug_assert_eq!(
                                order.len(),
                                old_depth - start + duplicated_sources.len()
                            );
                            apply_stack_operation(builder, &stack, &mut depth, operation);
                        }
                        _ => apply_stack_operation(builder, &stack, &mut depth, operation),
                    }
                    let (take, _) = stack_operation_order(operation);
                    let start = old_depth
                        .checked_sub(take)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    for (offset, variables) in stack
                        .get(start..depth)
                        .ok_or(CompileFailure::InvalidArtifact)?
                        .iter()
                        .copied()
                        .enumerate()
                    {
                        let index = start + offset;
                        let value = use_pair(builder, variables);
                        store_jsvalue_slot(builder, stack_base, index, value, layout)?;
                    }
                    if depth < old_depth {
                        for (offset, variables) in stack
                            .get(depth..old_depth)
                            .ok_or(CompileFailure::InvalidArtifact)?
                            .iter()
                            .copied()
                            .enumerate()
                        {
                            clear_pair(builder, variables, stack_base, depth + offset, layout)?;
                        }
                    }
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                }
                IrOp::Unary(operation) => {
                    let index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    if matches!(operation, UnaryOp::Plus | UnaryOp::LogicalNot) {
                        let state = helper_states
                            .next()
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        let slot = flat_stack_slot(ir, index)?;
                        let helper = if operation == UnaryOp::Plus {
                            qjs::JSJitHelperId_JS_JIT_HELPER_TO_NUMERIC
                        } else {
                            qjs::JSJitHelperId_JS_JIT_HELPER_TO_BOOL
                        };
                        invoke_helper!(helper, state, depth, &[slot, slot]);
                        reload_pair(builder, stack[index], stack_base, index, layout);
                        if operation == UnaryOp::LogicalNot {
                            let boolean = use_pair(builder, stack[index]);
                            let payload = builder.ins().bxor_imm(boolean.payload, 1);
                            let result = Pair {
                                payload,
                                tag: boolean.tag,
                            };
                            define_pair(builder, stack[index], result);
                            store_jsvalue_slot(builder, stack_base, index, result, layout)?;
                        }
                    } else {
                        let value = use_pair(builder, stack[index]);
                        let result = emit_unary(builder, value, operation);
                        define_pair(builder, stack[index], result);
                    }
                }
                IrOp::PostUnary(operation) => {
                    let value = use_pair(builder, stack[depth - 1]);
                    let result = emit_unary(builder, value, operation);
                    define_pair(builder, stack[depth], result);
                    depth += 1;
                }
                IrOp::LocalUnary { index, op } => {
                    let value = use_pair(builder, locals[index as usize]);
                    let result = emit_unary(builder, value, op);
                    define_pair(builder, locals[index as usize], result);
                }
                IrOp::AddLocal(index) => {
                    let left = use_pair(builder, locals[index as usize]);
                    let right = use_pair(builder, stack[depth - 1]);
                    let result = emit_binary(builder, left, right, BinaryOp::Add);
                    define_pair(builder, locals[index as usize], result);
                    depth -= 1;
                }
                IrOp::Binary(operation) => {
                    if operation == BinaryOp::Mod {
                        return Err(CompileFailure::InvalidArtifact);
                    }
                    let left_index = depth
                        .checked_sub(2)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let right_index = depth - 1;
                    let helper = match operation {
                        BinaryOp::Add => Some((qjs::JSJitHelperId_JS_JIT_HELPER_ADD_SLOW, None)),
                        BinaryOp::LessThan => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_LT),
                        )),
                        BinaryOp::LessThanOrEqual => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_LTE),
                        )),
                        BinaryOp::GreaterThan => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_GT),
                        )),
                        BinaryOp::GreaterThanOrEqual => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_GTE),
                        )),
                        BinaryOp::Equal => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_EQ),
                        )),
                        BinaryOp::NotEqual => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_NEQ),
                        )),
                        BinaryOp::StrictEqual => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_STRICT_EQ),
                        )),
                        BinaryOp::StrictNotEqual => Some((
                            qjs::JSJitHelperId_JS_JIT_HELPER_COMPARE_SLOW,
                            Some(qjs::JSJitCompareOp_JS_JIT_COMPARE_STRICT_NEQ),
                        )),
                        _ => None,
                    };
                    if let Some((helper, comparison)) = helper {
                        let state = helper_states
                            .next()
                            .ok_or(CompileFailure::InvalidArtifact)?;
                        let left = flat_stack_slot(ir, left_index)?;
                        let right = flat_stack_slot(ir, right_index)?;
                        if let Some(comparison) = comparison {
                            invoke_helper!(helper, state, depth, &[left, left, right, comparison]);
                        } else {
                            invoke_helper!(helper, state, depth, &[left, left, right]);
                        }
                        reload_pair(builder, stack[left_index], stack_base, left_index, layout);
                        reload_pair(builder, stack[right_index], stack_base, right_index, layout);
                        depth -= 1;
                        set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
                    } else {
                        let left = use_pair(builder, stack[left_index]);
                        let right = use_pair(builder, stack[right_index]);
                        let result = emit_binary(builder, left, right, operation);
                        depth -= 1;
                        define_pair(builder, stack[depth - 1], result);
                    }
                }
                IrOp::Jump(target) => {
                    builder.ins().jump(blocks[&target], &[]);
                    terminated = true;
                }
                IrOp::Branch { target, when_true } => {
                    let condition_index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let state = helper_states
                        .next()
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    let condition_slot = flat_stack_slot(ir, condition_index)?;
                    invoke_helper!(
                        qjs::JSJitHelperId_JS_JIT_HELPER_TO_BOOL,
                        state,
                        depth,
                        &[condition_slot, condition_slot]
                    );
                    reload_pair(
                        builder,
                        stack[condition_index],
                        stack_base,
                        condition_index,
                        layout,
                    );
                    let condition = use_pair(builder, stack[condition_index]);
                    depth -= 1;
                    let truthy = builder.ins().ireduce(types::I8, condition.payload);
                    clear_pair(
                        builder,
                        stack[condition_index],
                        stack_base,
                        condition_index,
                        layout,
                    )?;
                    set_visible_stack_depth(builder, frame, stack_base, depth, layout)?;
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
                    let result_index = depth
                        .checked_sub(1)
                        .ok_or(CompileFailure::InvalidArtifact)?;
                    materialize_frame(
                        builder,
                        frame,
                        arg_buf,
                        var_buf,
                        stack_base,
                        &arguments,
                        &locals,
                        &stack,
                        depth,
                        depth,
                        instruction.pc,
                        pointer_type,
                        layout,
                    )?;
                    let result = use_pair(builder, stack[result_index]);
                    store_jsvalue(builder, frame, layout.result, result, layout);
                    clear_pair(
                        builder,
                        stack[result_index],
                        stack_base,
                        result_index,
                        layout,
                    )?;
                    set_visible_stack_depth(builder, frame, stack_base, result_index, layout)?;
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
                    materialize_frame(
                        builder,
                        frame,
                        arg_buf,
                        var_buf,
                        stack_base,
                        &arguments,
                        &locals,
                        &stack,
                        depth,
                        depth,
                        instruction.pc,
                        pointer_type,
                        layout,
                    )?;
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
            if helper_states.next().is_some() {
                return Err(CompileFailure::InvalidArtifact);
            }
            if terminated {
                break;
            }
        }
        if !terminated {
            if let Some(next) = ir.blocks.get(block_index + 1) {
                builder.ins().jump(blocks[&next.start_pc], &[]);
            } else {
                return Err(CompileFailure::InvalidArtifact);
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
    builder.switch_to_block(invariant_trap);
    builder.ins().trap(TrapCode::unwrap_user(1));
    Ok(())
}

fn next_helper_state(
    states: &mut impl Iterator<Item = FrameStateId>,
) -> Result<FrameStateId, CompileFailure> {
    states.next().ok_or(CompileFailure::InvalidArtifact)
}

fn lower_new_array(
    builder: &mut FunctionBuilder<'_>,
    helpers: &HelperLowering<'_>,
    states: &mut impl Iterator<Item = FrameStateId>,
    depth: &mut usize,
    count: u16,
) -> Result<(), CompileFailure> {
    let count = usize::from(count);
    let input_base = depth
        .checked_sub(count)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let output_index = *depth;
    let output = flat_stack_slot(helpers.ir, output_index)?;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_NEW_ARRAY,
        next_helper_state(states)?,
        *depth,
        *depth,
        &[output],
    )?;
    reload_pair(
        builder,
        helpers.stack[output_index],
        helpers.stack_base,
        output_index,
        helpers.layout,
    );
    if count == 0 {
        *depth = input_base + 1;
        return helpers.set_depth(builder, *depth);
    }

    /*
     * Keep the array in the first logical input slot before invoking a
     * setter. The displaced first element occupies the second scratch slot,
     * so either setter exit has no owning value in scratch: SET_PROPERTY
     * consumes that element on both success and exception.
     */
    let displaced_index = output_index
        .checked_add(1)
        .ok_or(CompileFailure::ResourceLimit)?;
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        input_base,
        displaced_index,
        helpers.layout,
    )?;
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        output_index,
        input_base,
        helpers.layout,
    )?;
    let array = flat_stack_slot(helpers.ir, input_base)?;

    for index in 0..count {
        let value_index = if index == 0 {
            displaced_index
        } else {
            input_base + index
        };
        let value = flat_stack_slot(helpers.ir, value_index)?;
        let atom =
            (1_u32 << 31) | u32::try_from(index).map_err(|_| CompileFailure::ResourceLimit)?;
        helpers.invoke(
            builder,
            qjs::JSJitHelperId_JS_JIT_HELPER_SET_PROPERTY,
            next_helper_state(states)?,
            *depth + 2,
            *depth,
            &[array, atom, value],
        )?;
        reload_pair(
            builder,
            helpers.stack[value_index],
            helpers.stack_base,
            value_index,
            helpers.layout,
        );
        reload_pair(
            builder,
            helpers.stack[input_base],
            helpers.stack_base,
            input_base,
            helpers.layout,
        );
    }
    for index in (input_base + 1)..=displaced_index {
        clear_pair(
            builder,
            helpers.stack[index],
            helpers.stack_base,
            index,
            helpers.layout,
        )?;
    }
    *depth = input_base + 1;
    helpers.set_depth(builder, *depth)
}

fn lower_get_property(
    builder: &mut FunctionBuilder<'_>,
    helpers: &HelperLowering<'_>,
    states: &mut impl Iterator<Item = FrameStateId>,
    depth: usize,
    atom: u32,
) -> Result<(), CompileFailure> {
    let object_index = depth
        .checked_sub(1)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let output_index = depth;
    let displaced_index = depth.checked_add(1).ok_or(CompileFailure::ResourceLimit)?;
    let object = flat_stack_slot(helpers.ir, object_index)?;
    let output = flat_stack_slot(helpers.ir, output_index)?;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_GET_PROPERTY,
        next_helper_state(states)?,
        depth,
        depth,
        &[output, object, atom],
    )?;
    reload_pair(
        builder,
        helpers.stack[output_index],
        helpers.stack_base,
        output_index,
        helpers.layout,
    );
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        object_index,
        displaced_index,
        helpers.layout,
    )?;
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        output_index,
        object_index,
        helpers.layout,
    )?;
    let displaced = flat_stack_slot(helpers.ir, displaced_index)?;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
        next_helper_state(states)?,
        depth + 2,
        depth,
        &[displaced],
    )?;
    reload_pair(
        builder,
        helpers.stack[displaced_index],
        helpers.stack_base,
        displaced_index,
        helpers.layout,
    );
    helpers.set_depth(builder, depth)
}

fn lower_set_property(
    builder: &mut FunctionBuilder<'_>,
    helpers: &HelperLowering<'_>,
    states: &mut impl Iterator<Item = FrameStateId>,
    depth: &mut usize,
    atom: u32,
) -> Result<(), CompileFailure> {
    let object_index = depth
        .checked_sub(2)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let value_index = *depth - 1;
    let object = flat_stack_slot(helpers.ir, object_index)?;
    let value = flat_stack_slot(helpers.ir, value_index)?;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_SET_PROPERTY,
        next_helper_state(states)?,
        *depth,
        *depth,
        &[object, atom, value],
    )?;
    reload_pair(
        builder,
        helpers.stack[value_index],
        helpers.stack_base,
        value_index,
        helpers.layout,
    );
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
        next_helper_state(states)?,
        *depth,
        *depth,
        &[object],
    )?;
    reload_pair(
        builder,
        helpers.stack[object_index],
        helpers.stack_base,
        object_index,
        helpers.layout,
    );
    *depth = object_index;
    helpers.set_depth(builder, *depth)
}

fn lower_call(
    builder: &mut FunctionBuilder<'_>,
    helpers: &HelperLowering<'_>,
    states: &mut impl Iterator<Item = FrameStateId>,
    depth: &mut usize,
    argc: u16,
    has_this: bool,
) -> Result<(), CompileFailure> {
    let argc = usize::from(argc);
    let pop = argc + 1 + usize::from(has_this);
    let base = depth
        .checked_sub(pop)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let this_index = if has_this { base } else { *depth };
    let function_index = if has_this { base + 1 } else { base };
    let argv_index = function_index + 1;
    let output_index = if has_this { *depth } else { *depth + 1 };
    if !has_this {
        let undefined = constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
        define_pair(builder, helpers.stack[this_index], undefined);
    }
    let output = flat_stack_slot(helpers.ir, output_index)?;
    let function = flat_stack_slot(helpers.ir, function_index)?;
    let this_value = flat_stack_slot(helpers.ir, this_index)?;
    let argv = if argc == 0 {
        u32::MAX
    } else {
        flat_stack_slot(helpers.ir, argv_index)?
    };
    let call_live_depth = output_index;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_CALL,
        next_helper_state(states)?,
        call_live_depth,
        *depth,
        &[
            output,
            function,
            this_value,
            argv,
            u32::try_from(argc).map_err(|_| CompileFailure::ResourceLimit)?,
        ],
    )?;
    reload_pair(
        builder,
        helpers.stack[output_index],
        helpers.stack_base,
        output_index,
        helpers.layout,
    );
    // CALL borrows every input. The bytecode stack effect is separate and is
    // implemented with explicit FREE calls in QuickJS interpreter order.
    // Move the first input aside and install the result in its logical slot
    // before FREE can finalize or re-enter, leaving at most one scratch owner.
    let displaced_index = if has_this {
        output_index + 1
    } else {
        this_index
    };
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        base,
        displaced_index,
        helpers.layout,
    )?;
    move_stack_pair(
        builder,
        helpers.stack,
        helpers.stack_base,
        output_index,
        base,
        helpers.layout,
    )?;
    let displaced = flat_stack_slot(helpers.ir, displaced_index)?;
    helpers.invoke(
        builder,
        qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
        next_helper_state(states)?,
        *depth + 2,
        *depth,
        &[displaced],
    )?;
    reload_pair(
        builder,
        helpers.stack[displaced_index],
        helpers.stack_base,
        displaced_index,
        helpers.layout,
    );
    for index in (base + 1)..(base + pop) {
        let slot = flat_stack_slot(helpers.ir, index)?;
        helpers.invoke(
            builder,
            qjs::JSJitHelperId_JS_JIT_HELPER_FREE,
            next_helper_state(states)?,
            *depth + 2,
            *depth,
            &[slot],
        )?;
        reload_pair(
            builder,
            helpers.stack[index],
            helpers.stack_base,
            index,
            helpers.layout,
        );
    }
    for index in (base + 1)..=output_index {
        clear_pair(
            builder,
            helpers.stack[index],
            helpers.stack_base,
            index,
            helpers.layout,
        )?;
    }
    *depth = base + 1;
    helpers.set_depth(builder, *depth)
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

fn store_jsvalue_slot(
    builder: &mut FunctionBuilder<'_>,
    base: Value,
    index: usize,
    value: Pair,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let offset = index
        .checked_mul(mem::size_of::<qjs::JSValue>())
        .and_then(|offset| i32::try_from(offset).ok())
        .ok_or(CompileFailure::ResourceLimit)?;
    store_jsvalue(builder, base, offset, value, layout);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_frame(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    arg_buf: Value,
    var_buf: Value,
    stack_base: Value,
    arguments: &[PairVars],
    locals: &[PairVars],
    stack: &[PairVars],
    live_depth: usize,
    visible_depth: usize,
    bytecode_pc: u32,
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    if live_depth > visible_depth || visible_depth > stack.len() {
        return Err(CompileFailure::InvalidArtifact);
    }
    for (index, pair) in arguments.iter().copied().enumerate() {
        let value = use_pair(builder, pair);
        store_jsvalue_slot(builder, arg_buf, index, value, layout)?;
    }
    for (index, pair) in locals.iter().copied().enumerate() {
        let value = use_pair(builder, pair);
        store_jsvalue_slot(builder, var_buf, index, value, layout)?;
    }
    for (index, pair) in stack.iter().copied().take(live_depth).enumerate() {
        let value = use_pair(builder, pair);
        store_jsvalue_slot(builder, stack_base, index, value, layout)?;
    }
    let undefined = constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
    for (index, pair) in stack
        .iter()
        .copied()
        .enumerate()
        .take(visible_depth)
        .skip(live_depth)
    {
        define_pair(builder, pair, undefined);
        store_jsvalue_slot(builder, stack_base, index, undefined, layout)?;
    }
    let stack_bytes = visible_depth
        .checked_mul(mem::size_of::<qjs::JSValue>())
        .and_then(|bytes| i64::try_from(bytes).ok())
        .ok_or(CompileFailure::ResourceLimit)?;
    let stack_top = builder.ins().iadd_imm(stack_base, stack_bytes);
    let flags = MemFlags::new();
    builder
        .ins()
        .store(flags, stack_top, frame, layout.stack_top);
    let bytecode = builder
        .ins()
        .load(pointer_type, flags, frame, layout.bytecode_start);
    let pc = builder.ins().iadd_imm(bytecode, i64::from(bytecode_pc));
    builder.ins().store(flags, pc, frame, layout.pc);
    Ok(())
}

fn set_visible_stack_depth(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    stack_base: Value,
    depth: usize,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let bytes = depth
        .checked_mul(mem::size_of::<qjs::JSValue>())
        .and_then(|bytes| i64::try_from(bytes).ok())
        .ok_or(CompileFailure::ResourceLimit)?;
    let top = builder.ins().iadd_imm(stack_base, bytes);
    builder
        .ins()
        .store(MemFlags::new(), top, frame, layout.stack_top);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_helper_call(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    sret: Value,
    stack_base: Value,
    exception_depth: usize,
    signatures: &[cranelift_codegen::ir::SigRef],
    helper_id: usize,
    state: FrameStateId,
    arguments: &[Value],
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let signature = *signatures
        .get(helper_id)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let offset = *layout
        .helper_offsets
        .get(helper_id)
        .ok_or(CompileFailure::InvalidArtifact)?;
    let flags = MemFlags::new();
    let api = builder
        .ins()
        .load(pointer_type, flags, frame, layout.runtime_api);
    let helper = builder.ins().load(pointer_type, flags, api, offset);
    let mut params = Vec::with_capacity(arguments.len() + 1);
    params.push(frame);
    params.extend_from_slice(arguments);
    builder.set_srcloc(frame_state_source_loc(state)?);
    let call = builder.ins().call_indirect(signature, helper, &params);
    builder.set_srcloc(SourceLoc::default());
    let status = builder.inst_results(call)[0];
    let succeeded = builder.ins().icmp_imm(IntCC::Equal, status, 0);
    let continuation = builder.create_block();
    let exception = builder.create_block();
    builder
        .ins()
        .brif(succeeded, continuation, &[], exception, &[]);
    builder.seal_block(exception);
    builder.seal_block(continuation);
    builder.switch_to_block(exception);
    set_visible_stack_depth(builder, frame, stack_base, exception_depth, layout)?;
    emit_exit(
        builder,
        sret,
        qjs::JSJitExitKind_JS_JIT_EXIT_EXCEPTION,
        None,
        pointer_type,
    );
    builder.switch_to_block(continuation);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn invoke_frame_helper(
    builder: &mut FunctionBuilder<'_>,
    ir: &BaselineIr,
    frame: Value,
    sret: Value,
    arg_buf: Value,
    var_buf: Value,
    stack_base: Value,
    arguments: &[PairVars],
    locals: &[PairVars],
    stack: &[PairVars],
    live_depth: usize,
    exception_depth: usize,
    helper_signatures: &[cranelift_codegen::ir::SigRef],
    helper_id: usize,
    state: FrameStateId,
    helper_arguments: &[u32],
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let frame_state = ir.frame_states.get(state);
    let fixed_slots = usize::from(ir.argument_count) + usize::from(ir.local_count);
    let visible_depth = frame_state
        .slots
        .len()
        .checked_sub(fixed_slots)
        .ok_or(CompileFailure::InvalidArtifact)?;
    materialize_frame(
        builder,
        frame,
        arg_buf,
        var_buf,
        stack_base,
        arguments,
        locals,
        stack,
        live_depth,
        visible_depth,
        frame_state.pc,
        pointer_type,
        layout,
    )?;
    let mut values = Vec::with_capacity(helper_arguments.len() + 1);
    values.push(helper_u32(
        builder,
        u32::try_from(state.index()).map_err(|_| CompileFailure::ResourceLimit)?,
    ));
    values.extend(
        helper_arguments
            .iter()
            .copied()
            .map(|argument| helper_u32(builder, argument)),
    );
    emit_helper_call(
        builder,
        frame,
        sret,
        stack_base,
        exception_depth,
        helper_signatures,
        helper_id,
        state,
        &values,
        pointer_type,
        layout,
    )
}

fn helper_u32(builder: &mut FunctionBuilder<'_>, value: u32) -> Value {
    builder.ins().iconst(types::I32, i64::from(value))
}

fn flat_stack_slot(ir: &BaselineIr, index: usize) -> Result<u32, CompileFailure> {
    usize::from(ir.argument_count)
        .checked_add(usize::from(ir.local_count))
        .and_then(|base| base.checked_add(index))
        .and_then(|slot| u32::try_from(slot).ok())
        .ok_or(CompileFailure::ResourceLimit)
}

fn flat_argument_slot(index: u16) -> u32 {
    u32::from(index)
}

fn flat_local_slot(ir: &BaselineIr, index: u16) -> u32 {
    u32::from(ir.argument_count) + u32::from(index)
}

fn reload_pair(
    builder: &mut FunctionBuilder<'_>,
    variables: PairVars,
    base: Value,
    index: usize,
    layout: FrameLayout,
) {
    let value = load_jsvalue(builder, base, index, layout);
    define_pair(builder, variables, value);
}

fn move_stack_pair(
    builder: &mut FunctionBuilder<'_>,
    stack: &[PairVars],
    base: Value,
    source: usize,
    destination: usize,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    if source == destination {
        return Ok(());
    }
    let value = use_pair(builder, stack[source]);
    define_pair(builder, stack[destination], value);
    store_jsvalue_slot(builder, base, destination, value, layout)?;
    clear_pair(builder, stack[source], base, source, layout)
}

fn clear_pair(
    builder: &mut FunctionBuilder<'_>,
    variables: PairVars,
    base: Value,
    index: usize,
    layout: FrameLayout,
) -> Result<(), CompileFailure> {
    let undefined = constant_pair(builder, TaggedValue::new(0, qjs::JS_TAG_UNDEFINED as i64));
    define_pair(builder, variables, undefined);
    store_jsvalue_slot(builder, base, index, undefined, layout)
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

fn use_pair(builder: &mut FunctionBuilder<'_>, variables: PairVars) -> Pair {
    Pair {
        payload: builder.use_var(variables.payload),
        tag: builder.use_var(variables.tag),
    }
}

fn emit_entry_domain_guards(
    builder: &mut FunctionBuilder<'_>,
    arguments: &[PairVars],
    locals: &[PairVars],
    analysis: &EntryAnalysis,
    retry: Block,
) {
    for (root, requirements) in &analysis.requirements {
        debug_assert!(
            !requirements.uninitialized || !(requirements.numeric || requirements.initialized),
            "entry-domain analysis rejected conflicting requirements"
        );
        let variables = match *root {
            EntryRoot::Argument(index) => arguments[usize::from(index)],
            EntryRoot::Local(index) => locals[usize::from(index)],
        };
        let value = use_pair(builder, variables);
        let condition = if requirements.uninitialized {
            tag_is(builder, value.tag, qjs::JS_TAG_UNINITIALIZED)
        } else if requirements.numeric {
            emit_numeric_tag(builder, value.tag)
        } else {
            debug_assert!(requirements.initialized);
            builder.ins().icmp_imm(
                IntCC::NotEqual,
                value.tag,
                i64::from(qjs::JS_TAG_UNINITIALIZED),
            )
        };
        guard(builder, condition, retry);
    }
}

fn validate_live_stack_bounds(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    retry: Block,
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
    max_stack_slots: u16,
) {
    let flags = MemFlags::new();
    let stack_base = builder
        .ins()
        .load(pointer_type, flags, frame, layout.stack_base);
    let stack_top = builder
        .ins()
        .load(pointer_type, flags, frame, layout.stack_top);

    let base_non_null = builder.ins().icmp_imm(IntCC::NotEqual, stack_base, 0);
    let top_non_null = builder.ins().icmp_imm(IntCC::NotEqual, stack_top, 0);
    let non_null = builder.ins().band(base_non_null, top_non_null);
    guard(builder, non_null, retry);

    let combined = builder.ins().bor(stack_base, stack_top);
    let low_bits = builder.ins().band_imm(combined, 15);
    let aligned = builder.ins().icmp_imm(IntCC::Equal, low_bits, 0);
    guard(builder, aligned, retry);

    let ordered = builder
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, stack_top, stack_base);
    guard(builder, ordered, retry);

    let byte_len = builder.ins().isub(stack_top, stack_base);
    let trailing = builder.ins().band_imm(byte_len, 15);
    let whole_values = builder.ins().icmp_imm(IntCC::Equal, trailing, 0);
    guard(builder, whole_values, retry);

    let slot_count = builder.ins().ushr_imm(byte_len, 4);
    let slot_limit = builder
        .ins()
        .iconst(pointer_type, i64::from(max_stack_slots));
    let bounded = builder
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, slot_count, slot_limit);
    guard(builder, bounded, retry);
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

fn emit_frame_state_marker(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    invariant_trap: Block,
    state: FrameStateId,
) -> Result<(), CompileFailure> {
    let frame_is_non_null = builder.ins().icmp_imm(IntCC::NotEqual, frame, 0);
    let continuation = builder.create_block();
    builder.set_srcloc(frame_state_source_loc(state)?);
    // The entry ABI requires a non-null frame. Reasserting that invariant as
    // a control-flow edge is semantically inert for valid entries, cannot be
    // removed as a dead value, and gives each non-call state one exact machine
    // range without mutating the execution frame.
    builder
        .ins()
        .brif(frame_is_non_null, continuation, &[], invariant_trap, &[]);
    builder.set_srcloc(SourceLoc::default());
    builder.seal_block(continuation);
    builder.switch_to_block(continuation);
    Ok(())
}

fn emit_poll(
    builder: &mut FunctionBuilder<'_>,
    frame: Value,
    sret: Value,
    signature: cranelift_codegen::ir::SigRef,
    location: PollLocation,
    pointer_type: cranelift_codegen::ir::Type,
    layout: FrameLayout,
) {
    let flags = MemFlags::new();
    let api = builder
        .ins()
        .load(pointer_type, flags, frame, layout.runtime_api);
    let poll = builder.ins().load(
        pointer_type,
        flags,
        api,
        layout.helper_offsets[qjs::JSJitHelperId_JS_JIT_HELPER_POLL as usize],
    );
    builder.set_srcloc(location.source_location);
    let call = builder.ins().call_indirect(signature, poll, &[frame]);
    builder.set_srcloc(SourceLoc::default());
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
    let resume = builder
        .ins()
        .iadd_imm(bytecode, i64::from(location.bytecode_pc));
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

fn emit_numeric_tag(builder: &mut FunctionBuilder<'_>, tag: Value) -> Value {
    let is_int = tag_is(builder, tag, qjs::JS_TAG_INT);
    let is_float = tag_is(builder, tag, qjs::JS_TAG_FLOAT64);
    builder.ins().bor(is_int, is_float)
}

fn emit_numeric(builder: &mut FunctionBuilder<'_>, value: Pair) -> (Value, Value) {
    let is_int = tag_is(builder, value.tag, qjs::JS_TAG_INT);
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

fn emit_truthy(builder: &mut FunctionBuilder<'_>, value: Pair) -> Value {
    let is_null = tag_is(builder, value.tag, qjs::JS_TAG_NULL);
    let is_undefined = tag_is(builder, value.tag, qjs::JS_TAG_UNDEFINED);
    let is_float = tag_is(builder, value.tag, qjs::JS_TAG_FLOAT64);
    let empty = builder.ins().bor(is_null, is_undefined);
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

fn emit_unary(builder: &mut FunctionBuilder<'_>, value: Pair, operation: UnaryOp) -> Pair {
    match operation {
        UnaryOp::LogicalNot => {
            let truthy = emit_truthy(builder, value);
            let inverse = builder.ins().bxor_imm(truthy, 1);
            pair_from_bool(builder, inverse)
        }
        UnaryOp::BitNot => {
            let int = emit_to_i32(builder, value);
            let int = builder.ins().bnot(int);
            Pair {
                payload: builder.ins().sextend(types::I64, int),
                tag: builder.ins().iconst(types::I64, i64::from(qjs::JS_TAG_INT)),
            }
        }
        UnaryOp::Plus => {
            let _ = emit_numeric(builder, value);
            value
        }
        UnaryOp::Neg | UnaryOp::Increment | UnaryOp::Decrement => {
            let (is_int, number) = emit_numeric(builder, value);
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
) -> Pair {
    match operation {
        BinaryOp::BitAnd
        | BinaryOp::BitOr
        | BinaryOp::BitXor
        | BinaryOp::ShiftLeft
        | BinaryOp::ShiftRight
        | BinaryOp::ShiftRightUnsigned => emit_integer_binary(builder, left, right, operation),
        BinaryOp::LessThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThan
        | BinaryOp::GreaterThanOrEqual
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::StrictEqual
        | BinaryOp::StrictNotEqual => emit_comparison(builder, left, right, operation),
        _ => emit_arithmetic(builder, left, right, operation),
    }
}

fn emit_integer_binary(
    builder: &mut FunctionBuilder<'_>,
    left: Pair,
    right: Pair,
    operation: BinaryOp,
) -> Pair {
    let left = emit_to_i32(builder, left);
    let right = emit_to_i32(builder, right);
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
) -> Pair {
    debug_assert_ne!(operation, BinaryOp::Mod);
    let (left_int, left_float) = emit_numeric(builder, left);
    let (right_int, right_float) = emit_numeric(builder, right);
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

fn emit_to_i32(builder: &mut FunctionBuilder<'_>, value: Pair) -> Value {
    let (is_int, number) = emit_numeric(builder, value);
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
) -> Pair {
    let (_, left) = emit_numeric(builder, left);
    let (_, right) = emit_numeric(builder, right);
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
    let (take, order) = stack_operation_order(operation);
    let start = *depth - take;
    let values: Vec<Pair> = (0..take)
        .map(|index| use_pair(builder, stack[start + index]))
        .collect();
    for (destination, &source) in order.iter().enumerate() {
        define_pair(builder, stack[start + destination], values[source]);
    }
    *depth = start + order.len();
}

fn stack_operation_order(operation: StackOp) -> (usize, &'static [usize]) {
    match operation {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FrameStateTable, IrBlock, IrInstruction};

    fn instruction(pc: u32, op: IrOp) -> IrInstruction {
        IrInstruction {
            pc,
            frame_state: None,
            helper_states: Box::new([]),
            op,
        }
    }

    fn numeric_push() -> IrInstruction {
        instruction(0, IrOp::Push(TaggedValue::new(1, qjs::JS_TAG_INT as i64)))
    }

    fn linear_ir(mut prefix: Vec<IrInstruction>, operation: IrOp) -> BaselineIr {
        prefix.push(instruction(0, operation));
        if !matches!(prefix.last().map(|item| &item.op), Some(IrOp::Return)) {
            prefix.push(instruction(1, IrOp::ReturnUndefined));
        }
        BaselineIr {
            blocks: vec![IrBlock {
                start_pc: 0,
                stack_depth: 0,
                instructions: prefix,
            }],
            frame_states: FrameStateTable::default(),
            max_stack_depth: 16,
            argument_count: 4,
            local_count: 4,
        }
    }

    fn ir_op_variant(operation: &IrOp) -> &'static str {
        match operation {
            IrOp::Poll { .. } => "poll",
            IrOp::OsrLabel { .. } => "osr_label",
            IrOp::Nop => "nop",
            IrOp::Push(_) => "push",
            IrOp::ResolveConstant(_) => "resolve_constant",
            IrOp::NewObject => "new_object",
            IrOp::NewArrayFrom(_) => "new_array_from",
            IrOp::GetProperty(_) => "get_property",
            IrOp::SetProperty(_) => "set_property",
            IrOp::Call { .. } => "call",
            IrOp::GetArgument(_) => "get_argument",
            IrOp::GetLocal(_) => "get_local",
            IrOp::GetLocalChecked(_) => "get_local_checked",
            IrOp::GetLocalPair => "get_local_pair",
            IrOp::PutArgument { .. } => "put_argument",
            IrOp::PutLocal { .. } => "put_local",
            IrOp::PutLocalChecked { .. } => "put_local_checked",
            IrOp::SetLocalUninitialized(_) => "set_local_uninitialized",
            IrOp::Drop => "drop",
            IrOp::Stack(_) => "stack",
            IrOp::Unary(_) => "unary",
            IrOp::PostUnary(_) => "post_unary",
            IrOp::LocalUnary { .. } => "local_unary",
            IrOp::AddLocal(_) => "add_local",
            IrOp::Binary(_) => "binary",
            IrOp::Jump(_) => "jump",
            IrOp::Branch { .. } => "branch",
            IrOp::Return => "return",
            IrOp::ReturnUndefined => "return_undefined",
        }
    }

    #[test]
    fn entry_domain_analysis_exhaustively_handles_every_ir_op_variant() {
        let state = FrameStateId::from_index(0).unwrap();
        let mut cases = vec![
            linear_ir(
                Vec::new(),
                IrOp::Poll {
                    state,
                    kind: crate::ir::PollKind::Entry,
                },
            ),
            linear_ir(Vec::new(), IrOp::OsrLabel { state }),
            linear_ir(Vec::new(), IrOp::Nop),
            linear_ir(
                Vec::new(),
                IrOp::Push(TaggedValue::new(1, qjs::JS_TAG_INT as i64)),
            ),
            linear_ir(Vec::new(), IrOp::ResolveConstant(0)),
            linear_ir(Vec::new(), IrOp::NewObject),
            linear_ir(vec![numeric_push(), numeric_push()], IrOp::NewArrayFrom(2)),
            linear_ir(vec![numeric_push()], IrOp::GetProperty(1)),
            linear_ir(vec![numeric_push(), numeric_push()], IrOp::SetProperty(1)),
            linear_ir(
                vec![numeric_push(), numeric_push()],
                IrOp::Call {
                    argc: 1,
                    has_this: false,
                },
            ),
            linear_ir(Vec::new(), IrOp::GetArgument(0)),
            linear_ir(Vec::new(), IrOp::GetLocal(0)),
            linear_ir(Vec::new(), IrOp::GetLocalChecked(0)),
            linear_ir(Vec::new(), IrOp::GetLocalPair),
            linear_ir(
                vec![numeric_push()],
                IrOp::PutArgument {
                    index: 0,
                    keep: false,
                },
            ),
            linear_ir(
                vec![numeric_push()],
                IrOp::PutLocal {
                    index: 0,
                    keep: false,
                },
            ),
            linear_ir(
                vec![numeric_push()],
                IrOp::PutLocalChecked {
                    index: 0,
                    initialize: true,
                },
            ),
            linear_ir(Vec::new(), IrOp::SetLocalUninitialized(0)),
            linear_ir(vec![numeric_push()], IrOp::Drop),
            linear_ir(vec![numeric_push()], IrOp::Unary(UnaryOp::Plus)),
            linear_ir(vec![numeric_push()], IrOp::PostUnary(UnaryOp::Increment)),
            linear_ir(
                Vec::new(),
                IrOp::LocalUnary {
                    index: 0,
                    op: UnaryOp::Increment,
                },
            ),
            linear_ir(vec![numeric_push()], IrOp::AddLocal(0)),
            linear_ir(vec![numeric_push()], IrOp::Return),
            linear_ir(Vec::new(), IrOp::ReturnUndefined),
        ];
        for operation in [
            StackOp::Nip,
            StackOp::Nip1,
            StackOp::Dup,
            StackOp::Dup1,
            StackOp::Dup2,
            StackOp::Dup3,
            StackOp::Insert2,
            StackOp::Insert3,
            StackOp::Insert4,
            StackOp::Perm3,
            StackOp::Perm4,
            StackOp::Perm5,
            StackOp::Swap,
            StackOp::Swap2,
            StackOp::Rot3Left,
            StackOp::Rot3Right,
            StackOp::Rot4Left,
            StackOp::Rot5Left,
        ] {
            let (take, _) = stack_operation_order(operation);
            cases.push(linear_ir(
                (0..take).map(|_| numeric_push()).collect(),
                IrOp::Stack(operation),
            ));
        }
        for operation in [
            UnaryOp::Plus,
            UnaryOp::Neg,
            UnaryOp::Increment,
            UnaryOp::Decrement,
            UnaryOp::BitNot,
            UnaryOp::LogicalNot,
        ] {
            cases.push(linear_ir(vec![numeric_push()], IrOp::Unary(operation)));
        }
        for operation in [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Mod,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
            BinaryOp::ShiftLeft,
            BinaryOp::ShiftRight,
            BinaryOp::ShiftRightUnsigned,
            BinaryOp::LessThan,
            BinaryOp::LessThanOrEqual,
            BinaryOp::GreaterThan,
            BinaryOp::GreaterThanOrEqual,
            BinaryOp::Equal,
            BinaryOp::NotEqual,
            BinaryOp::StrictEqual,
            BinaryOp::StrictNotEqual,
        ] {
            cases.push(linear_ir(
                vec![numeric_push(), numeric_push()],
                IrOp::Binary(operation),
            ));
        }
        cases.push(BaselineIr {
            blocks: vec![
                IrBlock {
                    start_pc: 0,
                    stack_depth: 0,
                    instructions: vec![instruction(0, IrOp::Jump(1))],
                },
                IrBlock {
                    start_pc: 1,
                    stack_depth: 0,
                    instructions: vec![instruction(1, IrOp::ReturnUndefined)],
                },
            ],
            frame_states: FrameStateTable::default(),
            max_stack_depth: 0,
            argument_count: 0,
            local_count: 0,
        });
        cases.push(BaselineIr {
            blocks: vec![
                IrBlock {
                    start_pc: 0,
                    stack_depth: 0,
                    instructions: vec![
                        numeric_push(),
                        instruction(
                            0,
                            IrOp::Branch {
                                target: 2,
                                when_true: true,
                            },
                        ),
                    ],
                },
                IrBlock {
                    start_pc: 1,
                    stack_depth: 0,
                    instructions: vec![instruction(1, IrOp::ReturnUndefined)],
                },
                IrBlock {
                    start_pc: 2,
                    stack_depth: 0,
                    instructions: vec![instruction(2, IrOp::ReturnUndefined)],
                },
            ],
            frame_states: FrameStateTable::default(),
            max_stack_depth: 1,
            argument_count: 0,
            local_count: 0,
        });

        let mut seen = BTreeSet::new();
        for ir in cases {
            for operation in ir
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .map(|instruction| &instruction.op)
            {
                seen.insert(ir_op_variant(operation));
            }
            let analysis = analyze_entry_domains(&ir).expect("synthetic IR is structurally valid");
            let contains_mod = ir.blocks.iter().any(|block| {
                block
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction.op, IrOp::Binary(BinaryOp::Mod)))
            });
            assert_eq!(analysis.retry_before_entry, contains_mod, "{ir:?}");
        }
        assert_eq!(
            seen.len(),
            29,
            "every IrOp variant is represented: {seen:?}"
        );
    }
}
