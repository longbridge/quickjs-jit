use std::{io, ptr::NonNull};

use super::{CodeMemoryError, FaultInjection, MacJitMode, INJECTED_RAW_CODE};

pub(super) const SUPPORTED: bool = true;
pub(super) const INDIRECT_TARGET_ALIGNMENT: usize = 16;

pub(super) fn round_to_page(len: usize) -> Result<usize, CodeMemoryError> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(CodeMemoryError::MappingFailed {
            operation: "sysconf(_SC_PAGESIZE)",
            raw_code: raw_os_error(),
        });
    }
    let page_size = usize::try_from(page_size).map_err(|_| CodeMemoryError::SizeOverflow)?;
    len.checked_add(page_size - 1)
        .map(|rounded| rounded / page_size * page_size)
        .ok_or(CodeMemoryError::SizeOverflow)
}

#[derive(Debug)]
pub(super) struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub(super) fn allocate(
        len: usize,
        _owner_id: u64,
        _mac_jit_mode: MacJitMode,
    ) -> Result<Self, CodeMemoryError> {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(CodeMemoryError::MappingFailed {
                operation: "mmap(PROT_READ|PROT_WRITE)",
                raw_code: raw_os_error(),
            });
        }
        let ptr = NonNull::new(ptr.cast()).ok_or(CodeMemoryError::MappingFailed {
            operation: "mmap(PROT_READ|PROT_WRITE)",
            raw_code: 0,
        })?;
        Ok(Self { ptr, len })
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
        if fault == Some(FaultInjection::MacWriteProtection) {
            return Err(CodeMemoryError::WriteProtectionFailed {
                operation: "fault injection before platform write-protection transition",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.as_ptr(), bytes.len());
        }
        if fault == Some(FaultInjection::Protection) {
            return Err(CodeMemoryError::ProtectionFailed {
                operation: "fault injection before mprotect(PROT_READ|PROT_EXEC)",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        let result = unsafe {
            libc::mprotect(
                self.ptr.as_ptr().cast(),
                self.len,
                libc::PROT_READ | libc::PROT_EXEC,
            )
        };
        if result != 0 {
            return Err(CodeMemoryError::ProtectionFailed {
                operation: "mprotect(PROT_READ|PROT_EXEC)",
                raw_code: raw_os_error(),
            });
        }
        if fault == Some(FaultInjection::InstructionCache) {
            return Err(CodeMemoryError::InstructionCacheFlushFailed {
                operation: "fault injection before instruction-cache synchronization",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        unsafe {
            synchronize_instruction_cache(self.ptr.as_ptr(), bytes.len());
        }
        if fault == Some(FaultInjection::CfgRegistration) {
            return Err(CodeMemoryError::CfgRegistrationFailed {
                operation: "fault injection before indirect-target registration",
                raw_code: INJECTED_RAW_CODE,
            });
        }
        Ok(())
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

fn raw_os_error() -> i64 {
    i64::from(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
}

#[cfg(target_arch = "x86_64")]
unsafe fn synchronize_instruction_cache(_start: *mut u8, _len: usize) {
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_arch = "aarch64")]
unsafe fn synchronize_instruction_cache(start: *mut u8, len: usize) {
    use core::arch::asm;

    if len == 0 {
        return;
    }
    let mut ctr_el0: usize;
    unsafe {
        asm!("mrs {ctr}, ctr_el0", ctr = out(reg) ctr_el0, options(nomem, nostack));
    }
    let data_line = 4_usize << ((ctr_el0 >> 16) & 0xf);
    let instruction_line = 4_usize << (ctr_el0 & 0xf);
    let end = start as usize + len;

    let mut address = (start as usize) & !(data_line - 1);
    while address < end {
        unsafe {
            asm!("dc cvau, {address}", address = in(reg) address, options(nostack));
        }
        address += data_line;
    }
    unsafe {
        asm!("dsb ish", options(nostack));
    }

    address = (start as usize) & !(instruction_line - 1);
    while address < end {
        unsafe {
            asm!("ic ivau, {address}", address = in(reg) address, options(nostack));
        }
        address += instruction_line;
    }
    unsafe {
        asm!("dsb ish", "isb", options(nostack));
    }
}
