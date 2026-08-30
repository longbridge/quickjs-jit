use std::sync::atomic::{AtomicU64, Ordering};

use crate::runtime::{FunctionKey, Tier};

/// Full compatibility identity for one compiled artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactKey {
    pub runtime_id: u64,
    pub function_id: u64,
    pub generation: u64,
    pub tier: Tier,
    pub target_isa: u64,
    pub cpu_features: u64,
    pub abi_fingerprint: u64,
    pub source_revision: u64,
    pub opcode_fingerprint: u64,
    pub config_fingerprint: u64,
}

impl ArtifactKey {
    pub const fn with_tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct CodeAllocation {
    bytes: Box<[u8]>,
}

impl CodeAllocation {
    pub fn inert(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into_boxed_slice(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn is_executable(&self) -> bool {
        false
    }
}

/// Exact relocation encodings emitted by Cranelift 0.116.
///
/// Keeping the encoding in the artifact prevents the publisher from silently
/// treating target-relative relocations as absolute addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelocationKind {
    Abs4,
    Abs8,
    X86PCRel4,
    X86CallPCRel4,
    X86CallPLTRel4,
    X86GOTPCRel4,
    X86SecRel,
    Arm32Call,
    Arm64Call,
    S390xPCRel32Dbl,
    S390xPLTRel32Dbl,
    ElfX86_64TlsGd,
    MachOX86_64Tlv,
    MachOAarch64TlsAdrPage21,
    MachOAarch64TlsAdrPageOff12,
    Aarch64TlsDescAdrPage21,
    Aarch64TlsDescLd64Lo12,
    Aarch64TlsDescAddLo12,
    Aarch64TlsDescCall,
    Aarch64AdrGotPage21,
    Aarch64Ld64GotLo12Nc,
    RiscvCallPlt,
    RiscvTlsGdHi20,
    RiscvPCRelLo12I,
    RiscvGotHi20,
    S390xTlsGd64,
    S390xTlsGdCall,
    PulleyCallIndirectHost,
}

/// A relocation target retained symbolically until a writable allocation has
/// a stable address and the runtime has resolved external symbols.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelocationTarget {
    Absolute(u64),
    FunctionOffset(u32),
    Symbol(Box<str>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relocation {
    pub offset: u32,
    pub kind: RelocationKind,
    pub target: RelocationTarget,
    pub addend: i64,
}

impl Relocation {
    /// Backwards-compatible constructor for an absolute 64-bit relocation.
    pub const fn new(offset: u32, target: u64, addend: i64) -> Self {
        Self {
            offset,
            kind: RelocationKind::Abs8,
            target: RelocationTarget::Absolute(target),
            addend,
        }
    }

    pub const fn with_target(
        offset: u32,
        kind: RelocationKind,
        target: RelocationTarget,
        addend: i64,
    ) -> Self {
        Self {
            offset,
            kind,
            target,
            addend,
        }
    }

    /// Resolves the symbolic target without discarding the relocation kind.
    pub fn resolve_with(
        &self,
        mut resolve: impl FnMut(&RelocationTarget) -> Option<u64>,
    ) -> Result<ResolvedRelocation, RelocationResolveError> {
        let target = match self.target {
            RelocationTarget::Absolute(target) => target,
            _ => resolve(&self.target).ok_or(RelocationResolveError::UnresolvedTarget)?,
        };
        Ok(ResolvedRelocation::new(
            self.offset,
            self.kind,
            target,
            self.addend,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelocationResolveError {
    UnresolvedTarget,
}

/// A relocation whose target address has been explicitly resolved and is
/// ready for validation by the W^X publisher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedRelocation {
    pub offset: u32,
    pub kind: RelocationKind,
    pub target: u64,
    pub addend: i64,
}

impl ResolvedRelocation {
    pub const fn new(offset: u32, kind: RelocationKind, target: u64, addend: i64) -> Self {
        Self {
            offset,
            kind,
            target,
            addend,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackMap {
    pub code_offset: u32,
    pub live_slots: Box<[u16]>,
}

impl StackMap {
    pub fn new(code_offset: u32, live_slots: Vec<u16>) -> Self {
        Self {
            code_offset,
            live_slots: live_slots.into_boxed_slice(),
        }
    }
}

/// Native unwind format represented by serialized Cranelift 0.116 metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnwindKind {
    SystemV,
    WindowsX64,
    WindowsArm64,
}

/// Version-pinned unwind metadata retained with compiled code.
///
/// The encoding is Cranelift 0.116's serde representation. Keeping it opaque
/// here avoids making the cache depend on Cranelift when the compiler feature
/// is disabled while preserving the complete unwind plan for registration by
/// a later installation stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnwindMetadata {
    kind: UnwindKind,
    frame_size: u32,
    encoding: Box<[u8]>,
}

impl UnwindMetadata {
    pub fn new(kind: UnwindKind, frame_size: u32, encoding: Vec<u8>) -> Self {
        Self {
            kind,
            frame_size,
            encoding: encoding.into_boxed_slice(),
        }
    }

    pub const fn kind(&self) -> UnwindKind {
        self.kind
    }

    pub const fn frame_size(&self) -> u32 {
        self.frame_size
    }

    pub fn encoding(&self) -> &[u8] {
        &self.encoding
    }
}

/// Exact native location represented by a retained logical frame state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameStateLocationKind {
    /// The code offset is the return address of one machine call site.
    CallReturn,
    /// The code offset is the start of one emitted non-call marker range.
    Marker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameState {
    pub code_offset: u32,
    pub bytecode_pc: u32,
    pub slots: Box<[u16]>,
    pub location_kind: FrameStateLocationKind,
    pub source_location: u32,
    pub source_start: u32,
    pub source_end: u32,
}

impl FrameState {
    pub fn new(code_offset: u32, bytecode_pc: u32, slots: Vec<u16>) -> Self {
        Self::with_location(
            code_offset,
            bytecode_pc,
            slots,
            FrameStateLocationKind::Marker,
            0,
            code_offset,
            code_offset.saturating_add(1),
        )
    }

    pub fn with_location(
        code_offset: u32,
        bytecode_pc: u32,
        slots: Vec<u16>,
        location_kind: FrameStateLocationKind,
        source_location: u32,
        source_start: u32,
        source_end: u32,
    ) -> Self {
        Self {
            code_offset,
            bytecode_pc,
            slots: slots.into_boxed_slice(),
            location_kind,
            source_location,
            source_start,
            source_end,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactDependency {
    pub function: FunctionKey,
}

impl ArtifactDependency {
    pub const fn new(function: FunctionKey) -> Self {
        Self { function }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BenefitSnapshot {
    pub executions: u64,
    pub score: u64,
}

#[derive(Debug, Default)]
struct BenefitCounters {
    executions: AtomicU64,
    score: AtomicU64,
}

impl BenefitCounters {
    fn record(&self, score: u64) {
        let _ = self
            .executions
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(1))
            });
        let _ = self
            .score
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(score))
            });
    }

    fn snapshot(&self) -> BenefitSnapshot {
        BenefitSnapshot {
            executions: self.executions.load(Ordering::Acquire),
            score: self.score.load(Ordering::Acquire),
        }
    }
}

/// Compiler output. Task 5 allocations contain inert owned bytes only.
#[derive(Debug)]
pub struct CompiledArtifact {
    key: ArtifactKey,
    code: CodeAllocation,
    relocations: Box<[Relocation]>,
    unwind_metadata: Option<UnwindMetadata>,
    stack_maps: Box<[StackMap]>,
    frame_states: Box<[FrameState]>,
    dependencies: Box<[ArtifactDependency]>,
    benefit: BenefitCounters,
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    relocatable: Option<Box<crate::compiler::baseline::RelocatableCode>>,
    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    published: Option<crate::compiler::baseline::PublishedBaselineCode>,
    #[cfg(any(test, feature = "test-support"))]
    fake: bool,
}

impl CompiledArtifact {
    pub fn empty(key: ArtifactKey) -> Self {
        Self::from_parts(
            key,
            CodeAllocation::inert(Vec::new()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    pub fn from_parts(
        key: ArtifactKey,
        code: CodeAllocation,
        relocations: Vec<Relocation>,
        stack_maps: Vec<StackMap>,
        frame_states: Vec<FrameState>,
        dependencies: Vec<ArtifactDependency>,
    ) -> Self {
        Self {
            key,
            code,
            relocations: relocations.into_boxed_slice(),
            unwind_metadata: None,
            stack_maps: stack_maps.into_boxed_slice(),
            frame_states: frame_states.into_boxed_slice(),
            dependencies: dependencies.into_boxed_slice(),
            benefit: BenefitCounters::default(),
            #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
            relocatable: None,
            #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
            published: None,
            #[cfg(any(test, feature = "test-support"))]
            fake: false,
        }
    }

    pub const fn key(&self) -> ArtifactKey {
        self.key
    }

    pub const fn code(&self) -> &CodeAllocation {
        &self.code
    }

    pub fn relocations(&self) -> &[Relocation] {
        &self.relocations
    }

    pub fn unwind_metadata(&self) -> Option<&UnwindMetadata> {
        self.unwind_metadata.as_ref()
    }

    pub fn with_unwind_metadata(mut self, metadata: Option<UnwindMetadata>) -> Self {
        self.unwind_metadata = metadata;
        self
    }

    pub fn stack_maps(&self) -> &[StackMap] {
        &self.stack_maps
    }

    pub fn frame_states(&self) -> &[FrameState] {
        &self.frame_states
    }

    pub fn dependencies(&self) -> &[ArtifactDependency] {
        &self.dependencies
    }

    pub fn benefit(&self) -> BenefitSnapshot {
        self.benefit.snapshot()
    }

    pub fn charge_bytes(&self) -> Option<usize> {
        fn add_slice<T>(total: usize, len: usize) -> Option<usize> {
            total.checked_add(len.checked_mul(core::mem::size_of::<T>())?)
        }

        let mut total = self.code.bytes().len();
        total = add_slice::<Relocation>(total, self.relocations.len())?;
        for relocation in &self.relocations {
            if let RelocationTarget::Symbol(symbol) = &relocation.target {
                total = total.checked_add(symbol.len())?;
            }
        }
        if let Some(unwind) = &self.unwind_metadata {
            total = total.checked_add(core::mem::size_of::<UnwindMetadata>())?;
            total = total.checked_add(unwind.encoding.len())?;
        }
        total = add_slice::<StackMap>(total, self.stack_maps.len())?;
        for map in &self.stack_maps {
            total = add_slice::<u16>(total, map.live_slots.len())?;
        }
        total = add_slice::<FrameState>(total, self.frame_states.len())?;
        for state in &self.frame_states {
            total = add_slice::<u16>(total, state.slots.len())?;
        }
        add_slice::<ArtifactDependency>(total, self.dependencies.len())
    }

    pub fn code_bytes(&self) -> usize {
        self.code.bytes().len()
    }

    pub fn metadata_bytes(&self) -> Option<usize> {
        self.charge_bytes()?.checked_sub(self.code_bytes())
    }

    pub(crate) fn record_benefit(&self, score: u64) {
        self.benefit.record(score);
    }

    pub(crate) fn benefit_score(&self) -> u64 {
        self.benefit.score.load(Ordering::Acquire)
    }

    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub(crate) fn with_relocatable(
        mut self,
        code: crate::compiler::baseline::RelocatableCode,
    ) -> Self {
        self.relocatable = Some(Box::new(code));
        self
    }

    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub(crate) fn publish_relocatable(&mut self) -> Result<(), crate::platform::CodeMemoryError> {
        if let Some(code) = self.relocatable.take() {
            self.published = Some(code.publish()?);
        }
        Ok(())
    }

    #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
    pub fn published(&self) -> Option<&crate::compiler::baseline::PublishedBaselineCode> {
        self.published.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fake(tier: Tier) -> Self {
        Self {
            key: ArtifactKey {
                runtime_id: 0,
                function_id: 0,
                generation: 0,
                tier,
                target_isa: 0,
                cpu_features: 0,
                abi_fingerprint: 0,
                source_revision: 0,
                opcode_fingerprint: 0,
                config_fingerprint: 0,
            },
            code: CodeAllocation::inert(Vec::new()),
            relocations: Box::new([]),
            unwind_metadata: None,
            stack_maps: Box::new([]),
            frame_states: Box::new([]),
            dependencies: Box::new([]),
            benefit: BenefitCounters::default(),
            #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
            relocatable: None,
            #[cfg(all(feature = "compiler", not(target_family = "wasm")))]
            published: None,
            fake: true,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn bind_fake(mut self, key: ArtifactKey) -> Self {
        if self.fake && self.key.tier == key.tier {
            self.key = key;
            self.fake = false;
        }
        self
    }
}
