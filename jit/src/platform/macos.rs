use std::{
    collections::BTreeMap,
    ffi::c_void,
    io,
    ptr::NonNull,
    sync::{Mutex, MutexGuard, OnceLock},
};

use super::{CodeMemoryError, FaultInjection, MacJitMode, MacJitWriteContext, INJECTED_RAW_CODE};

pub(super) const SUPPORTED: bool = true;
pub(super) const INDIRECT_TARGET_ALIGNMENT: usize = 16;

const PROCESS_HEAP_BYTES: usize = 256 * 1024 * 1024;

pub(super) fn round_to_page(len: usize) -> Result<usize, CodeMemoryError> {
    let page_size = page_size()?;
    len.checked_add(page_size - 1)
        .map(|rounded| rounded / page_size * page_size)
        .ok_or(CodeMemoryError::SizeOverflow)
}

#[derive(Debug)]
pub(super) struct Mapping {
    ptr: NonNull<u8>,
    offset: usize,
    len: usize,
    owner_id: u64,
    mode: MacJitMode,
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub(super) fn allocate(
        len: usize,
        owner_id: u64,
        mode: MacJitMode,
    ) -> Result<Self, CodeMemoryError> {
        let mut heap = heap()?;
        let offset = heap.allocate(len, owner_id)?;
        let ptr = NonNull::new((heap.base + offset) as *mut u8).ok_or(
            CodeMemoryError::MappingFailed {
                operation: "suballocate process MAP_JIT heap",
                raw_code: i64::from(libc::ENOMEM),
            },
        )?;
        Ok(Self {
            ptr,
            offset,
            len,
            owner_id,
            mode,
        })
    }

    pub(super) fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub(super) fn publish(
        &mut self,
        bytes: &[u8],
        _indirect_targets: &[usize],
        fault: Option<FaultInjection>,
    ) -> Result<(), CodeMemoryError> {
        if fault == Some(FaultInjection::Protection) {
            return Err(CodeMemoryError::ProtectionFailed {
                operation: "fault injection before macOS JIT publication",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        if fault == Some(FaultInjection::MacWriteProtection) {
            return Err(CodeMemoryError::WriteProtectionFailed {
                operation: "fault injection before macOS JIT write transition",
                raw_code: INJECTED_RAW_CODE,
            });
        }

        match self.mode {
            MacJitMode::ThreadWriteProtect => unsafe {
                libc::pthread_jit_write_protect_np(0);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.as_ptr(), bytes.len());
                libc::pthread_jit_write_protect_np(1);
            },
            MacJitMode::AllowListCallback(callback) => {
                let mut context = MacJitWriteContext {
                    destination: self.ptr.as_ptr(),
                    source: bytes.as_ptr(),
                    len: bytes.len(),
                };
                let status = unsafe {
                    pthread_jit_write_with_callback_np(
                        Some(callback),
                        (&mut context as *mut MacJitWriteContext).cast(),
                    )
                };
                if status != 0 {
                    return Err(CodeMemoryError::WriteCallbackRejected {
                        operation: "pthread_jit_write_with_callback_np",
                        raw_code: i64::from(status),
                    });
                }
            }
        }

        if fault == Some(FaultInjection::InstructionCache) {
            return Err(CodeMemoryError::InstructionCacheFlushFailed {
                operation: "fault injection before sys_icache_invalidate",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        unsafe {
            sys_icache_invalidate(self.ptr.as_ptr().cast(), bytes.len());
        }
        if fault == Some(FaultInjection::CfgRegistration) {
            return Err(CodeMemoryError::CfgRegistrationFailed {
                operation: "fault injection before indirect-target publication",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        Ok(())
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        if let Ok(mut heap) = heap() {
            heap.release(self.offset, self.len, self.owner_id);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct HeapAllocation {
    len: usize,
    owner_id: u64,
}

#[derive(Debug)]
struct ProcessHeap {
    base: usize,
    len: usize,
    free: BTreeMap<usize, usize>,
    allocations: BTreeMap<usize, HeapAllocation>,
}

impl ProcessHeap {
    fn create() -> Result<Mutex<Self>, CodeMemoryError> {
        let len = round_to_page(PROCESS_HEAP_BYTES)?;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_JIT,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let raw_code = raw_os_error();
            if raw_code == i64::from(libc::EPERM) || raw_code == i64::from(libc::EACCES) {
                return Err(CodeMemoryError::MissingEntitlement {
                    operation: "mmap(MAP_JIT)",
                    raw_code,
                });
            }
            return Err(CodeMemoryError::MappingFailed {
                operation: "mmap(MAP_JIT)",
                raw_code,
            });
        }
        let Some(base) = NonNull::<u8>::new(ptr.cast()) else {
            return Err(CodeMemoryError::MappingFailed {
                operation: "mmap(MAP_JIT)",
                raw_code: 0,
            });
        };
        let mut free = BTreeMap::new();
        free.insert(0, len);
        Ok(Mutex::new(Self {
            base: base.as_ptr() as usize,
            len,
            free,
            allocations: BTreeMap::new(),
        }))
    }

    fn allocate(&mut self, len: usize, owner_id: u64) -> Result<usize, CodeMemoryError> {
        let candidate = self
            .free
            .iter()
            .find_map(|(&offset, &available)| (available >= len).then_some((offset, available)));
        let Some((offset, available)) = candidate else {
            return Err(CodeMemoryError::MappingFailed {
                operation: "suballocate process MAP_JIT heap",
                raw_code: i64::from(libc::ENOMEM),
            });
        };
        self.free.remove(&offset);
        if available > len {
            self.free.insert(offset + len, available - len);
        }
        self.allocations
            .insert(offset, HeapAllocation { len, owner_id });
        Ok(offset)
    }

    fn release(&mut self, offset: usize, len: usize, owner_id: u64) {
        let Some(allocation) = self.allocations.remove(&offset) else {
            return;
        };
        if allocation.len != len || allocation.owner_id != owner_id {
            self.allocations.insert(offset, allocation);
            return;
        }

        let mut start = offset;
        let mut free_len = len;
        if let Some((&previous, &previous_len)) = self.free.range(..offset).next_back() {
            if previous + previous_len == offset {
                self.free.remove(&previous);
                start = previous;
                free_len += previous_len;
            }
        }
        if let Some((&next, &next_len)) = self.free.range(start..).next() {
            if start + free_len == next {
                self.free.remove(&next);
                free_len += next_len;
            }
        }
        debug_assert!(start + free_len <= self.len);
        self.free.insert(start, free_len);
    }
}

fn heap() -> Result<MutexGuard<'static, ProcessHeap>, CodeMemoryError> {
    static HEAP: OnceLock<Result<Mutex<ProcessHeap>, CodeMemoryError>> = OnceLock::new();
    let heap = HEAP
        .get_or_init(ProcessHeap::create)
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(heap.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

fn page_size() -> Result<usize, CodeMemoryError> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(CodeMemoryError::MappingFailed {
            operation: "sysconf(_SC_PAGESIZE)",
            raw_code: raw_os_error(),
        });
    }
    usize::try_from(page_size).map_err(|_| CodeMemoryError::SizeOverflow)
}

fn raw_os_error() -> i64 {
    i64::from(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
}

unsafe extern "C" {
    fn pthread_jit_write_with_callback_np(
        callback: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
        context: *mut c_void,
    ) -> i32;
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
}
