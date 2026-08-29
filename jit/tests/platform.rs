#![cfg(all(
    target_endian = "little",
    any(
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
    )
))]

#[path = "support/host_asm.rs"]
mod host_asm;

use rquickjs::{Context, Runtime};
use rquickjs_jit::{
    code_cache::{Relocation, RelocationKind, RelocationTarget, ResolvedRelocation},
    platform::{CodeAllocator, CodeMemoryError, FaultInjection, MacJitPolicy},
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

fn unique_runtime_owner() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0x4000_0000_0000_0000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn publish_is_one_way_and_code_executes() {
    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(4096).unwrap();
    host_asm::write_return_42(&mut writable).unwrap();
    let executable = writable.publish().unwrap();
    let result = unsafe { executable.call0_i32() };
    assert_eq!(result, 42);
    assert!(!executable.is_writable());
}

#[test]
fn quota_is_enforced_before_mapping() {
    let allocator = CodeAllocator::with_limit(4096).unwrap();
    let _first = allocator.allocate(4096).unwrap();
    assert!(matches!(
        allocator.allocate(1),
        Err(CodeMemoryError::LimitExceeded)
    ));
}

#[test]
fn relocations_are_validated_as_a_batch_before_writes() {
    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(32).unwrap();
    writable.write(0, &[0xaa; 16]).unwrap();
    let before = writable.bytes().to_vec();
    let relocations = [
        ResolvedRelocation::new(0, RelocationKind::Abs8, 0x1020, -0x20),
        ResolvedRelocation::new(28, RelocationKind::Abs8, 0x2000, 0),
    ];

    assert!(matches!(
        writable.apply_relocations(&relocations),
        Err(CodeMemoryError::RelocationOutOfBounds { offset: 28, .. })
    ));
    assert_eq!(writable.bytes(), before);

    writable
        .apply_relocations(&[ResolvedRelocation::new(
            0,
            RelocationKind::Abs8,
            0x1020,
            -0x20,
        )])
        .unwrap();
    assert_eq!(&writable.bytes()[..8], &0x1000_u64.to_le_bytes());
}

#[test]
fn symbolic_relocation_preserves_kind_and_symbol_through_publication() {
    let relocation = Relocation::with_target(
        8,
        RelocationKind::Abs8,
        RelocationTarget::Symbol("qjsjit_interrupt_poll".into()),
        -0x20,
    );
    assert_eq!(relocation.kind, RelocationKind::Abs8);
    assert_eq!(
        relocation.target,
        RelocationTarget::Symbol("qjsjit_interrupt_poll".into())
    );
    let resolved = relocation
        .resolve_with(|target| match target {
            RelocationTarget::Symbol(name) if name.as_ref() == "qjsjit_interrupt_poll" => {
                Some(0x1020)
            }
            _ => None,
        })
        .unwrap();

    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(32).unwrap();
    writable.apply_relocations(&[resolved]).unwrap();
    assert_eq!(&writable.bytes()[8..16], &0x1000_u64.to_le_bytes());
    writable.declare_indirect_targets(&[0]).unwrap();
    let executable = writable.publish().unwrap();
    assert_eq!(executable.len(), 32);
}

#[test]
fn entry_acquisition_rejects_undeclared_and_misaligned_offsets() {
    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(64).unwrap();
    host_asm::write_return(&mut writable, 0, 41).unwrap();
    host_asm::write_return(&mut writable, 16, 42).unwrap();
    writable.declare_indirect_targets(&[16]).unwrap();
    let executable = writable.publish().unwrap();

    assert!(matches!(
        executable.entry(1),
        Err(CodeMemoryError::InvalidIndirectTarget { offset: 1, .. })
    ));
    assert!(matches!(
        executable.entry(32),
        Err(CodeMemoryError::UndeclaredIndirectTarget { offset: 32 })
    ));
    let second = executable.entry(16).unwrap();
    assert_eq!(unsafe { second.call0_i32() }, 42);
}

#[test]
fn executable_clones_pin_the_mapping_and_quota() {
    let allocator = CodeAllocator::with_limit(4096).unwrap();
    let mut writable = allocator.allocate(32).unwrap();
    host_asm::write_return_42(&mut writable).unwrap();
    let executable = writable.publish().unwrap();
    let pin = executable.clone();
    drop(executable);

    assert_eq!(allocator.reserved_bytes(), 4096);
    assert_eq!(unsafe { pin.call0_i32() }, 42);
    drop(pin);
    assert_eq!(allocator.reserved_bytes(), 0);
}

#[test]
fn platform_faults_disable_native_code_without_breaking_interpretation() {
    for fault in [
        FaultInjection::PageSize,
        FaultInjection::Mapping,
        FaultInjection::Protection,
        FaultInjection::InstructionCache,
        FaultInjection::CfgRegistration,
        FaultInjection::MacWriteProtection,
    ] {
        let allocator = CodeAllocator::with_fault_injection(4096, fault).unwrap();
        let result = match allocator.allocate(32) {
            Ok(mut writable) => {
                host_asm::write_return_42(&mut writable).unwrap();
                writable.publish().map(|_| ())
            }
            Err(error) => Err(error),
        };
        let error = result.unwrap_err();
        assert!(matches!(
            (fault, error),
            (
                FaultInjection::PageSize,
                CodeMemoryError::PageSizeFailed { .. }
            ) | (
                FaultInjection::Mapping,
                CodeMemoryError::MappingFailed { .. }
            ) | (
                FaultInjection::Protection,
                CodeMemoryError::ProtectionFailed { .. }
            ) | (
                FaultInjection::InstructionCache,
                CodeMemoryError::InstructionCacheFlushFailed { .. },
            ) | (
                FaultInjection::CfgRegistration,
                CodeMemoryError::CfgRegistrationFailed { .. }
            ) | (
                FaultInjection::MacWriteProtection,
                CodeMemoryError::WriteProtectionFailed { .. }
            )
        ));
        assert!(!allocator.native_enabled());
        assert_eq!(allocator.reserved_bytes(), 0);
        assert!(matches!(
            allocator.allocate(1),
            Err(CodeMemoryError::NativeDisabled)
        ));

        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        let value = context.with(|ctx| ctx.eval::<i32, _>("1 + 1")).unwrap();
        assert_eq!(value, 2);
    }
}

#[test]
fn runtime_owners_have_explicit_mac_policy_and_independent_quotas() {
    assert!(matches!(
        MacJitPolicy::default(),
        MacJitPolicy::ThreadWriteProtect
    ));
    let first_owner = unique_runtime_owner();
    let second_owner = unique_runtime_owner();
    let first = CodeAllocator::for_runtime_with_mac_policy(
        first_owner,
        4096,
        MacJitPolicy::ThreadWriteProtect,
    )
    .unwrap();
    let second = CodeAllocator::for_runtime(second_owner, 4096).unwrap();
    assert_eq!(first.owner_id(), first_owner);
    assert_eq!(second.owner_id(), second_owner);
    let _first_mapping = first.allocate(4096).unwrap();
    let _second_mapping = second.allocate(4096).unwrap();
}

#[test]
fn duplicate_runtime_allocators_share_quota_and_configuration() {
    let owner = unique_runtime_owner();
    let first = CodeAllocator::for_runtime(owner, 4096).unwrap();
    let second = CodeAllocator::for_runtime(owner, 4096).unwrap();
    let _mapping = first.allocate(4096).unwrap();

    assert_eq!(second.reserved_bytes(), 4096);
    assert!(matches!(
        second.allocate(1),
        Err(CodeMemoryError::LimitExceeded)
    ));
    assert!(matches!(
        CodeAllocator::for_runtime(owner, 8192),
        Err(CodeMemoryError::OwnerConfigurationMismatch { owner_id }) if owner_id == owner
    ));
}

#[test]
fn disabling_failure_rejects_writable_code_from_the_previous_epoch() {
    let owner = unique_runtime_owner();
    let first = CodeAllocator::for_runtime_with_fault_injection(
        owner,
        4096 * 2,
        FaultInjection::Protection,
    )
    .unwrap();
    let second = CodeAllocator::for_runtime_with_fault_injection(
        owner,
        4096 * 2,
        FaultInjection::Protection,
    )
    .unwrap();
    let mut failing = first.allocate(32).unwrap();
    let mut stale = second.allocate(32).unwrap();
    host_asm::write_return_42(&mut failing).unwrap();
    host_asm::write_return_42(&mut stale).unwrap();

    assert!(matches!(
        failing.publish(),
        Err(CodeMemoryError::ProtectionFailed { .. })
    ));
    assert!(matches!(
        stale.publish(),
        Err(CodeMemoryError::NativeDisabled)
    ));
    assert!(!first.native_enabled());
    assert!(!second.native_enabled());
    assert_eq!(first.reserved_bytes(), 0);
}

#[test]
fn disabling_failure_wins_against_a_concurrent_publish() {
    let owner = unique_runtime_owner();
    let allocator = CodeAllocator::for_runtime_with_fault_injection(
        owner,
        4096 * 2,
        FaultInjection::Protection,
    )
    .unwrap();
    let mut first = allocator.allocate(32).unwrap();
    let mut second = allocator.allocate(32).unwrap();
    host_asm::write_return_42(&mut first).unwrap();
    host_asm::write_return_42(&mut second).unwrap();
    let barrier = Arc::new(Barrier::new(2));

    let first_barrier = Arc::clone(&barrier);
    let first_publish = std::thread::spawn(move || {
        first_barrier.wait();
        first.publish().unwrap_err()
    });
    let second_publish = std::thread::spawn(move || {
        barrier.wait();
        second.publish().unwrap_err()
    });
    let errors = [
        first_publish.join().unwrap(),
        second_publish.join().unwrap(),
    ];

    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(error, CodeMemoryError::ProtectionFailed { .. }))
            .count(),
        1
    );
    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(error, CodeMemoryError::NativeDisabled))
            .count(),
        1
    );
    assert!(!allocator.native_enabled());
    assert_eq!(allocator.reserved_bytes(), 0);
}

#[test]
fn mac_jit_allowlist_policy_has_an_explicit_unsafe_bootstrap_boundary() {
    let bootstrap: unsafe fn(MacJitPolicy) -> Result<(), CodeMemoryError> =
        CodeAllocator::bootstrap_mac_jit_policy;
    let _ = bootstrap;
}

#[test]
fn separately_published_versions_execute_across_threads() {
    for value in [41, 42] {
        let executable = std::thread::spawn(move || {
            let allocator = CodeAllocator::for_host().unwrap();
            let mut writable = allocator.allocate(32).unwrap();
            host_asm::write_return(&mut writable, 0, value).unwrap();
            writable.publish().unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(
            std::thread::spawn(move || unsafe { executable.call0_i32() })
                .join()
                .unwrap(),
            value
        );
    }
}

#[cfg(all(target_endian = "little", target_os = "linux"))]
#[test]
fn linux_mapping_transitions_from_rw_to_rx_without_rwx() {
    fn permissions(address: *const u8) -> String {
        let address = address as usize;
        std::fs::read_to_string("/proc/self/maps")
            .unwrap()
            .lines()
            .find_map(|line| {
                let (range, rest) = line.split_once(' ')?;
                let (start, end) = range.split_once('-')?;
                let start = usize::from_str_radix(start, 16).ok()?;
                let end = usize::from_str_radix(end, 16).ok()?;
                (start <= address && address < end)
                    .then(|| rest.split_whitespace().next().unwrap().to_owned())
            })
            .expect("mapping appears in /proc/self/maps")
    }

    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(32).unwrap();
    let writable_address = writable.as_ptr();
    assert_eq!(permissions(writable_address), "rw-p");
    host_asm::write_return_42(&mut writable).unwrap();
    let executable = writable.publish().unwrap();
    assert_eq!(permissions(executable.as_ptr()), "r-xp");
}

#[cfg(all(target_endian = "little", target_os = "windows"))]
#[test]
fn windows_mapping_transitions_from_rw_to_rx_without_rwx() {
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
        PAGE_READWRITE,
    };

    fn protection(address: *const u8) -> u32 {
        let mut information = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQuery(
                address.cast(),
                &mut information,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        assert_eq!(queried, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        information.Protect
    }

    let allocator = CodeAllocator::for_host().unwrap();
    let mut writable = allocator.allocate(32).unwrap();
    let writable_address = writable.as_ptr();
    assert_eq!(protection(writable_address) & 0xff, PAGE_READWRITE);
    assert_ne!(protection(writable_address) & 0xff, PAGE_EXECUTE_READWRITE);
    host_asm::write_return_42(&mut writable).unwrap();
    let executable = writable.publish().unwrap();
    assert_eq!(protection(executable.as_ptr()) & 0xff, PAGE_EXECUTE_READ);
    assert_ne!(
        protection(executable.as_ptr()) & 0xff,
        PAGE_EXECUTE_READWRITE
    );
}
