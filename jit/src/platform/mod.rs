//! W^X executable-memory allocation for supported desktop targets.

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use linux as backend;

#[cfg(all(
    target_os = "macos",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod macos;
#[cfg(all(
    target_os = "macos",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use macos as backend;

#[cfg(all(
    target_os = "windows",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod windows;
#[cfg(all(
    target_os = "windows",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use windows as backend;

#[cfg(not(any(
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "windows",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
mod unsupported;
#[cfg(not(any(
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "windows",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
use unsupported as backend;

use std::{
    ffi::c_void,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

use crate::code_cache::Relocation;

/// Deterministic platform failure used to verify interpreter fallback paths.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultInjection {
    Mapping,
    Protection,
    InstructionCache,
    CfgRegistration,
    MacWriteProtection,
}

/// The embedder-selected policy for writing to the macOS process JIT heap.
#[derive(Clone, Copy, Debug, Default)]
pub enum MacJitMode {
    /// Toggle the calling thread with `pthread_jit_write_protect_np`.
    #[default]
    ThreadWriteProtect,
    /// Route the copy through an embedder callback in Apple's JIT allowlist.
    ///
    /// The callback receives a pointer to [`MacJitWriteContext`]. It must
    /// validate the context and perform the copy, returning zero on success.
    AllowListCallback(unsafe extern "C" fn(*mut c_void) -> i32),
}

/// Copy request passed to a macOS JIT write-allowlist callback.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MacJitWriteContext {
    pub destination: *mut u8,
    pub source: *const u8,
    pub len: usize,
}

pub(super) const INJECTED_RAW_CODE: i64 = -1;

/// Failures while reserving, writing, or publishing executable memory.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CodeMemoryError {
    UnsupportedPlatform,
    LimitExceeded,
    InvalidSize,
    SizeOverflow,
    WriteOutOfBounds {
        offset: usize,
        len: usize,
        allocation_len: usize,
    },
    MappingFailed {
        operation: &'static str,
        raw_code: i64,
    },
    ProtectionFailed {
        operation: &'static str,
        raw_code: i64,
    },
    InstructionCacheFlushFailed {
        operation: &'static str,
        raw_code: i64,
    },
    CfgRegistrationFailed {
        operation: &'static str,
        raw_code: i64,
    },
    WriteProtectionFailed {
        operation: &'static str,
        raw_code: i64,
    },
    MissingEntitlement {
        operation: &'static str,
        raw_code: i64,
    },
    WriteCallbackRejected {
        operation: &'static str,
        raw_code: i64,
    },
    RelocationOutOfBounds {
        offset: usize,
        width: usize,
        allocation_len: usize,
    },
    RelocationValueOutOfRange {
        offset: usize,
        target: u64,
        addend: i64,
    },
    InvalidIndirectTarget {
        offset: usize,
        alignment: usize,
        allocation_len: usize,
    },
    UndeclaredIndirectTarget {
        offset: usize,
    },
    NativeDisabled,
}

impl fmt::Display for CodeMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                f.write_str("executable memory is unsupported on this platform")
            }
            Self::LimitExceeded => f.write_str("executable-memory limit exceeded"),
            Self::InvalidSize => f.write_str("executable-memory allocation size must be nonzero"),
            Self::SizeOverflow => f.write_str("executable-memory allocation size overflowed"),
            Self::WriteOutOfBounds {
                offset,
                len,
                allocation_len,
            } => write!(
                f,
                "code write at {offset} with length {len} exceeds allocation length {allocation_len}"
            ),
            Self::MappingFailed {
                operation,
                raw_code,
            }
            | Self::ProtectionFailed {
                operation,
                raw_code,
            }
            | Self::InstructionCacheFlushFailed {
                operation,
                raw_code,
            }
            | Self::CfgRegistrationFailed {
                operation,
                raw_code,
            }
            | Self::WriteProtectionFailed {
                operation,
                raw_code,
            }
            | Self::MissingEntitlement {
                operation,
                raw_code,
            }
            | Self::WriteCallbackRejected {
                operation,
                raw_code,
            } => write!(f, "{operation} failed with OS error {raw_code}"),
            Self::RelocationOutOfBounds {
                offset,
                width,
                allocation_len,
            } => write!(
                f,
                "relocation at {offset} with width {width} exceeds allocation length {allocation_len}"
            ),
            Self::RelocationValueOutOfRange {
                offset,
                target,
                addend,
            } => write!(
                f,
                "relocation at {offset} cannot represent target {target} plus addend {addend}"
            ),
            Self::InvalidIndirectTarget {
                offset,
                alignment,
                allocation_len,
            } => write!(
                f,
                "indirect target {offset} is outside allocation length {allocation_len} or not {alignment}-byte aligned"
            ),
            Self::UndeclaredIndirectTarget { offset } => {
                write!(f, "indirect target {offset} was not declared")
            }
            Self::NativeDisabled => {
                f.write_str("native installation is disabled for this allocator owner")
            }
        }
    }
}

impl std::error::Error for CodeMemoryError {}

#[derive(Debug)]
struct AllocatorState {
    limit: usize,
    reserved: AtomicUsize,
    native_enabled: AtomicBool,
    fault: Option<FaultInjection>,
    owner_id: u64,
    mac_jit_mode: MacJitMode,
}

/// A logical executable-memory owner with an independent quota.
#[derive(Clone, Debug)]
pub struct CodeAllocator {
    state: Arc<AllocatorState>,
}

impl CodeAllocator {
    /// Creates an allocator for the current supported host.
    pub fn for_host() -> Result<Self, CodeMemoryError> {
        Self::new(next_owner_id(), usize::MAX, MacJitMode::default(), None)
    }

    /// Creates an allocator whose live mappings may use at most `limit` bytes.
    pub fn with_limit(limit: usize) -> Result<Self, CodeMemoryError> {
        Self::new(next_owner_id(), limit, MacJitMode::default(), None)
    }

    /// Creates an allocator for one runtime owner with the default macOS mode.
    pub fn for_runtime(owner_id: u64, limit: usize) -> Result<Self, CodeMemoryError> {
        Self::new(owner_id, limit, MacJitMode::default(), None)
    }

    /// Creates an allocator for one runtime and an explicit macOS write mode.
    pub fn for_runtime_with_mac_mode(
        owner_id: u64,
        limit: usize,
        mac_jit_mode: MacJitMode,
    ) -> Result<Self, CodeMemoryError> {
        Self::new(owner_id, limit, mac_jit_mode, None)
    }

    /// Creates an allocator that fails one platform operation deterministically.
    #[doc(hidden)]
    pub fn with_fault_injection(
        limit: usize,
        fault: FaultInjection,
    ) -> Result<Self, CodeMemoryError> {
        Self::new(next_owner_id(), limit, MacJitMode::default(), Some(fault))
    }

    fn new(
        owner_id: u64,
        limit: usize,
        mac_jit_mode: MacJitMode,
        fault: Option<FaultInjection>,
    ) -> Result<Self, CodeMemoryError> {
        if !backend::SUPPORTED {
            return Err(CodeMemoryError::UnsupportedPlatform);
        }
        Ok(Self {
            state: Arc::new(AllocatorState {
                limit,
                reserved: AtomicUsize::new(0),
                native_enabled: AtomicBool::new(true),
                fault,
                owner_id,
                mac_jit_mode,
            }),
        })
    }

    /// Allocates writable, non-executable storage.
    pub fn allocate(&self, len: usize) -> Result<WritableCode, CodeMemoryError> {
        if !self.native_enabled() {
            return Err(CodeMemoryError::NativeDisabled);
        }
        if len == 0 {
            return Err(CodeMemoryError::InvalidSize);
        }
        let mapped_len = backend::round_to_page(len)?;
        let reservation = Reservation::acquire(Arc::clone(&self.state), mapped_len)?;
        if self.state.fault == Some(FaultInjection::Mapping) {
            self.state.native_enabled.store(false, Ordering::Release);
            return Err(CodeMemoryError::MappingFailed {
                operation: "fault injection before executable-memory mapping",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        let mapping = match backend::Mapping::allocate(
            mapped_len,
            self.state.owner_id,
            self.state.mac_jit_mode,
        ) {
            Ok(mapping) => mapping,
            Err(error) => {
                self.state.native_enabled.store(false, Ordering::Release);
                return Err(error);
            }
        };
        Ok(WritableCode {
            mapping: Some(mapping),
            bytes: vec![0; len],
            reservation: Some(reservation),
            state: Arc::clone(&self.state),
            indirect_targets: vec![0],
        })
    }

    pub fn reserved_bytes(&self) -> usize {
        self.state.reserved.load(Ordering::Acquire)
    }

    pub fn native_enabled(&self) -> bool {
        self.state.native_enabled.load(Ordering::Acquire)
    }

    pub fn owner_id(&self) -> u64 {
        self.state.owner_id
    }
}

#[derive(Debug)]
struct Reservation {
    state: Arc<AllocatorState>,
    bytes: usize,
}

impl Reservation {
    fn acquire(state: Arc<AllocatorState>, bytes: usize) -> Result<Self, CodeMemoryError> {
        let mut current = state.reserved.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes)
                .ok_or(CodeMemoryError::LimitExceeded)?;
            if next > state.limit {
                return Err(CodeMemoryError::LimitExceeded);
            }
            match state.reserved.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self { state, bytes }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.state.reserved.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A writable, non-executable code allocation that has not been published.
#[derive(Debug)]
pub struct WritableCode {
    mapping: Option<backend::Mapping>,
    bytes: Vec<u8>,
    reservation: Option<Reservation>,
    state: Arc<AllocatorState>,
    indirect_targets: Vec<usize>,
}

impl WritableCode {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.mapping
            .as_ref()
            .expect("live writable mapping")
            .as_ptr()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), CodeMemoryError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(CodeMemoryError::WriteOutOfBounds {
                offset,
                len: bytes.len(),
                allocation_len: self.bytes.len(),
            })?;
        let allocation_len = self.bytes.len();
        let Some(destination) = self.bytes.get_mut(offset..end) else {
            return Err(CodeMemoryError::WriteOutOfBounds {
                offset,
                len: bytes.len(),
                allocation_len,
            });
        };
        destination.copy_from_slice(bytes);
        Ok(())
    }

    /// Applies absolute 64-bit relocations after validating the complete batch.
    pub fn apply_relocations(&mut self, relocations: &[Relocation]) -> Result<(), CodeMemoryError> {
        const WIDTH: usize = size_of::<u64>();
        let mut writes = Vec::with_capacity(relocations.len());
        for relocation in relocations {
            let offset = relocation.offset as usize;
            let end = offset
                .checked_add(WIDTH)
                .ok_or(CodeMemoryError::RelocationOutOfBounds {
                    offset,
                    width: WIDTH,
                    allocation_len: self.bytes.len(),
                })?;
            if end > self.bytes.len() {
                return Err(CodeMemoryError::RelocationOutOfBounds {
                    offset,
                    width: WIDTH,
                    allocation_len: self.bytes.len(),
                });
            }
            let value = i128::from(relocation.target) + i128::from(relocation.addend);
            let value =
                u64::try_from(value).map_err(|_| CodeMemoryError::RelocationValueOutOfRange {
                    offset,
                    target: relocation.target,
                    addend: relocation.addend,
                })?;
            writes.push((offset, value.to_le_bytes()));
        }
        for (offset, bytes) in writes {
            self.bytes[offset..offset + WIDTH].copy_from_slice(&bytes);
        }
        Ok(())
    }

    /// Declares additional entry offsets that may be acquired indirectly.
    pub fn declare_indirect_targets(&mut self, offsets: &[usize]) -> Result<(), CodeMemoryError> {
        for &offset in offsets {
            validate_indirect_target(offset, self.bytes.len())?;
        }
        self.indirect_targets.extend_from_slice(offsets);
        self.indirect_targets.sort_unstable();
        self.indirect_targets.dedup();
        Ok(())
    }

    /// Copies staged bytes and permanently changes the mapping from RW to RX.
    pub fn publish(mut self) -> Result<ExecutableCode, CodeMemoryError> {
        let mut mapping = self.mapping.take().expect("publish called once");
        for &offset in &self.indirect_targets {
            validate_indirect_target(offset, self.bytes.len())?;
        }
        if let Err(error) = mapping.publish(&self.bytes, &self.indirect_targets, self.state.fault) {
            self.state.native_enabled.store(false, Ordering::Release);
            return Err(error);
        }
        let reservation = self.reservation.take().expect("live reservation");
        Ok(ExecutableCode {
            allocation: Arc::new(ExecutableAllocation {
                mapping,
                logical_len: self.bytes.len(),
                indirect_targets: self.indirect_targets.into_boxed_slice(),
                _reservation: reservation,
            }),
            entry_offset: 0,
        })
    }
}

#[derive(Debug)]
struct ExecutableAllocation {
    mapping: backend::Mapping,
    logical_len: usize,
    indirect_targets: Box<[usize]>,
    _reservation: Reservation,
}

/// Immutable executable code. Clones pin the underlying RX mapping.
#[derive(Clone, Debug)]
pub struct ExecutableCode {
    allocation: Arc<ExecutableAllocation>,
    entry_offset: usize,
}

impl ExecutableCode {
    pub fn len(&self) -> usize {
        self.allocation.logical_len
    }

    pub fn is_empty(&self) -> bool {
        self.allocation.logical_len == 0
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.allocation.mapping.as_ptr()
    }

    pub const fn is_writable(&self) -> bool {
        false
    }

    /// Acquires a previously declared and platform-aligned indirect entry.
    pub fn entry(&self, offset: usize) -> Result<Self, CodeMemoryError> {
        validate_indirect_target(offset, self.allocation.logical_len)?;
        if self
            .allocation
            .indirect_targets
            .binary_search(&offset)
            .is_err()
        {
            return Err(CodeMemoryError::UndeclaredIndirectTarget { offset });
        }
        Ok(Self {
            allocation: Arc::clone(&self.allocation),
            entry_offset: offset,
        })
    }

    /// Calls the declared entry at offset zero while holding an allocation pin.
    ///
    /// # Safety
    ///
    /// The published bytes must implement the platform C ABI and return `i32`.
    pub unsafe fn call0_i32(&self) -> i32 {
        let pin = Arc::clone(&self.allocation);
        assert!(pin
            .indirect_targets
            .binary_search(&self.entry_offset)
            .is_ok());
        let entry_ptr = unsafe { pin.mapping.as_ptr().add(self.entry_offset) };
        let entry: unsafe extern "C" fn() -> i32 = unsafe { std::mem::transmute(entry_ptr) };
        let result = unsafe { entry() };
        drop(pin);
        result
    }
}

fn validate_indirect_target(offset: usize, allocation_len: usize) -> Result<(), CodeMemoryError> {
    if offset >= allocation_len || !offset.is_multiple_of(backend::INDIRECT_TARGET_ALIGNMENT) {
        return Err(CodeMemoryError::InvalidIndirectTarget {
            offset,
            alignment: backend::INDIRECT_TARGET_ALIGNMENT,
            allocation_len,
        });
    }
    Ok(())
}

fn next_owner_id() -> u64 {
    static NEXT_OWNER_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_OWNER_ID.fetch_add(1, Ordering::Relaxed)
}
