#[path = "support/host_asm.rs"]
mod host_asm;

use rquickjs::{Context, Runtime};
use rquickjs_jit::{
    code_cache::Relocation,
    platform::{CodeAllocator, CodeMemoryError, FaultInjection, MacJitMode},
};

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
        Relocation::new(0, 0x1020, -0x20),
        Relocation::new(28, 0x2000, 0),
    ];

    assert!(matches!(
        writable.apply_relocations(&relocations),
        Err(CodeMemoryError::RelocationOutOfBounds { offset: 28, .. })
    ));
    assert_eq!(writable.bytes(), before);

    writable
        .apply_relocations(&[Relocation::new(0, 0x1020, -0x20)])
        .unwrap();
    assert_eq!(&writable.bytes()[..8], &0x1000_u64.to_le_bytes());
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
        MacJitMode::default(),
        MacJitMode::ThreadWriteProtect
    ));
    let first =
        CodeAllocator::for_runtime_with_mac_mode(11, 4096, MacJitMode::ThreadWriteProtect).unwrap();
    let second = CodeAllocator::for_runtime(12, 4096).unwrap();
    assert_eq!(first.owner_id(), 11);
    assert_eq!(second.owner_id(), 12);
    let _first_mapping = first.allocate(4096).unwrap();
    let _second_mapping = second.allocate(4096).unwrap();
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "windows")]
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
