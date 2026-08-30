use rquickjs_jit::abi::{AbiInfo, ABI_MAJOR, ABI_MINOR};

const BUNDLED_TARGETS: [&str; 9] = [
    "x86_64-unknown-linux-gnu.rs",
    "aarch64-unknown-linux-gnu.rs",
    "x86_64-unknown-linux-musl.rs",
    "aarch64-unknown-linux-musl.rs",
    "x86_64-apple-darwin.rs",
    "aarch64-apple-darwin.rs",
    "x86_64-pc-windows-gnu.rs",
    "x86_64-pc-windows-msvc.rs",
    "aarch64-pc-windows-msvc.rs",
];

fn jit_declarations(source: &str) -> String {
    let lines: Vec<_> = source.lines().collect();
    assert!(lines.iter().any(|line| line.starts_with("pub type size_t")));
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("pub const __JS_ATOM_NULL"))
            .count(),
        1
    );
    let first_struct = lines
        .iter()
        .position(|line| line.starts_with("pub struct JSJitFunctionId"))
        .expect("JSJitFunctionId declaration");
    let atoms = lines
        .iter()
        .position(|line| line.starts_with("pub const __JS_ATOM_NULL"))
        .expect("atom declarations");
    let mut normalized = String::from("pub type size_t = NORMALIZED;\n");
    for line in &lines {
        if line.starts_with("pub const QJSJIT_ABI_") {
            normalized.push_str(line);
            normalized.push('\n');
        }
    }
    for line in &lines[first_struct - 2..atoms] {
        normalized.push_str(line);
        normalized.push('\n');
    }
    normalized
}

fn bundled_binding(target: &str) -> String {
    let binding_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sys/src/bindings");
    std::fs::read_to_string(binding_dir.join(target)).unwrap()
}

#[test]
#[allow(clippy::absurd_extreme_comparisons)]
fn linked_abi_matches_rust_contract() {
    let info = AbiInfo::linked().expect("ABI info");
    assert_eq!(info.major(), ABI_MAJOR);
    assert!(info.minor() >= ABI_MINOR);
    assert_eq!(info.pointer_width(), usize::BITS as u8);
    assert_eq!(info.little_endian(), cfg!(target_endian = "little"));
}

#[test]
fn abi_minor_additions_are_appended_after_the_v1_0_tail() {
    use rquickjs_core::qjs;

    let previous_tail = std::mem::offset_of!(qjs::JSJitABIInfo, backend_vtable_layout_fingerprint)
        + std::mem::size_of::<u64>();
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitABIInfo, exec_frame_layout_fingerprint),
        previous_tail
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitABIInfo, exit_layout_fingerprint),
        previous_tail + std::mem::size_of::<u64>()
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitABIInfo, runtime_api_layout_fingerprint),
        previous_tail + 2 * std::mem::size_of::<u64>()
    );
}

#[repr(C)]
struct AbiInfoV1_0 {
    struct_size: u32,
    major: u16,
    minor: u16,
    pointer_width: u8,
    little_endian: u8,
    value_size: u16,
    source_revision: u64,
    opcode_fingerprint: u64,
    value_layout_fingerprint: u64,
    build_feature_flags: u64,
    build_fingerprint: u64,
    abi_info_layout_fingerprint: u64,
    function_id_layout_fingerprint: u64,
    hot_event_layout_fingerprint: u64,
    function_snapshot_layout_fingerprint: u64,
    entry_handle_layout_fingerprint: u64,
    backend_vtable_layout_fingerprint: u64,
}

#[repr(C)]
struct AbiInfoV1_1 {
    prefix: AbiInfoV1_0,
    exec_frame_layout_fingerprint: u64,
    exit_layout_fingerprint: u64,
}

#[repr(C)]
struct AbiInfoV1_2 {
    prefix: AbiInfoV1_1,
    runtime_api_layout_fingerprint: u64,
}

#[repr(C)]
struct Guarded<T> {
    prefix: T,
    canary: [u8; 16],
}

#[test]
fn linked_abi_query_fills_every_old_prefix_without_touching_canaries() {
    use rquickjs_core::qjs;

    assert_eq!(
        std::mem::size_of::<AbiInfoV1_0>(),
        std::mem::offset_of!(qjs::JSJitABIInfo, exec_frame_layout_fingerprint)
    );
    assert_eq!(
        std::mem::size_of::<AbiInfoV1_1>(),
        std::mem::offset_of!(qjs::JSJitABIInfo, runtime_api_layout_fingerprint)
    );

    let mut v1_0: Guarded<AbiInfoV1_0> = unsafe { std::mem::zeroed() };
    v1_0.prefix.struct_size = std::mem::size_of::<AbiInfoV1_0>() as u32;
    v1_0.canary = [0xa5; 16];
    let status = unsafe {
        qjs::JS_GetJitABIInfo((&mut v1_0.prefix as *mut AbiInfoV1_0).cast::<qjs::JSJitABIInfo>())
    };
    assert_eq!(status, qjs::JS_JIT_BACKEND_OK);
    assert_eq!(
        v1_0.prefix.struct_size as usize,
        std::mem::size_of::<qjs::JSJitABIInfo>()
    );
    assert_eq!(v1_0.prefix.major, ABI_MAJOR);
    assert_eq!(v1_0.prefix.minor, ABI_MINOR);
    assert_eq!(v1_0.canary, [0xa5; 16]);

    let mut v1_1: Guarded<AbiInfoV1_1> = unsafe { std::mem::zeroed() };
    v1_1.prefix.prefix.struct_size = std::mem::size_of::<AbiInfoV1_1>() as u32;
    v1_1.canary = [0x5a; 16];
    let status = unsafe {
        qjs::JS_GetJitABIInfo((&mut v1_1.prefix as *mut AbiInfoV1_1).cast::<qjs::JSJitABIInfo>())
    };
    assert_eq!(status, qjs::JS_JIT_BACKEND_OK);
    assert_eq!(
        v1_1.prefix.prefix.struct_size as usize,
        std::mem::size_of::<qjs::JSJitABIInfo>()
    );
    assert_eq!(v1_1.prefix.prefix.major, ABI_MAJOR);
    assert_eq!(v1_1.prefix.prefix.minor, ABI_MINOR);
    assert_eq!(v1_1.canary, [0x5a; 16]);

    assert_eq!(
        std::mem::size_of::<AbiInfoV1_2>(),
        std::mem::offset_of!(qjs::JSJitABIInfo, helper_table_fingerprint)
    );
    let mut v1_2: Guarded<AbiInfoV1_2> = unsafe { std::mem::zeroed() };
    v1_2.prefix.prefix.prefix.struct_size = std::mem::size_of::<AbiInfoV1_2>() as u32;
    v1_2.canary = [0x3c; 16];
    let status = unsafe {
        qjs::JS_GetJitABIInfo((&mut v1_2.prefix as *mut AbiInfoV1_2).cast::<qjs::JSJitABIInfo>())
    };
    assert_eq!(status, qjs::JS_JIT_BACKEND_OK);
    assert_eq!(
        v1_2.prefix.prefix.prefix.struct_size as usize,
        std::mem::size_of::<qjs::JSJitABIInfo>()
    );
    assert_eq!(v1_2.prefix.prefix.prefix.minor, ABI_MINOR);
    assert_eq!(v1_2.canary, [0x3c; 16]);
}

#[test]
fn interrupt_runtime_api_is_a_versioned_exec_frame_tail_extension() {
    use rquickjs_core::qjs;

    assert_eq!(qjs::QJSJIT_RUNTIME_API_MAJOR, 1);
    assert_eq!(qjs::QJSJIT_RUNTIME_API_MINOR, 1);
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitExecFrame, runtime_api),
        std::mem::offset_of!(qjs::JSJitExecFrame, entry)
            + std::mem::size_of::<qjs::JSJitEntryHandle>()
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitRuntimeAPI, interrupt_poll),
        8
    );
}

#[test]
fn helper_abi_is_one_canonical_versioned_table_in_c_bindgen_and_rust() {
    use std::ffi::CStr;

    use rquickjs_core::qjs;

    let expected = [
        ("POLL", 0_u8, 0_u8),
        (
            "DUP",
            2,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        ("FREE", 1, 0),
        (
            "RESOLVE_CONST",
            1,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "TO_NUMERIC",
            2,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "TO_BOOL",
            2,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "ADD_SLOW",
            3,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "COMPARE_SLOW",
            3,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "GET_PROPERTY",
            2,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        ("SET_PROPERTY", 2, 0),
        (
            "CALL",
            4,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "NEW_ARRAY",
            1,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
        (
            "NEW_OBJECT",
            1,
            qjs::JSJitHelperOwnership_JS_JIT_HELPER_OWNED as u8,
        ),
    ];

    let mut count = 0_u32;
    let mut fingerprint = 0_u64;
    let native = unsafe { qjs::JS_JitGetHelperTable(&mut count, &mut fingerprint) };
    assert!(!native.is_null());
    assert_eq!(count as usize, expected.len());
    assert_eq!(count as usize, qjs::QJSJIT_GENERATED_HELPERS.len());
    assert_eq!(fingerprint, qjs::QJSJIT_GENERATED_HELPER_FINGERPRINT);

    for (index, ((name, value_arity, output_ownership), generated)) in expected
        .iter()
        .zip(qjs::QJSJIT_GENERATED_HELPERS)
        .enumerate()
    {
        let native = unsafe { &*native.add(index) };
        assert_eq!(native.id as usize, index);
        assert_eq!(generated.id as usize, index);
        assert_eq!(
            unsafe { CStr::from_ptr(native.name) }.to_str().unwrap(),
            *name
        );
        assert_eq!(generated.name, *name);
        assert_eq!(native.value_arity, *value_arity);
        assert_eq!(generated.value_arity, *value_arity);
        assert_eq!(native.output_ownership, *output_ownership);
        assert_eq!(generated.output_ownership, *output_ownership);
        assert_eq!(native.flags, generated.flags);
        assert_eq!(native.abi_type_count, generated.abi_types.len() as u8);
        assert_eq!(
            &native.abi_types[..usize::from(native.abi_type_count)],
            generated.abi_types
        );
        assert_eq!(
            &native.value_ownership[..usize::from(native.value_arity)],
            generated.value_ownership
        );
    }
}

#[test]
fn helper_abi_fields_are_append_only_tails() {
    use rquickjs_core::qjs;

    assert_eq!(qjs::QJSJIT_ABI_MINOR, 3);
    assert_eq!(qjs::QJSJIT_RUNTIME_API_MAJOR, 1);
    assert_eq!(qjs::QJSJIT_RUNTIME_API_MINOR, 1);
    assert_eq!(qjs::QJSJIT_HELPER_ABI_VERSION, 1);
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitEntryHandle, stack_map_count),
        std::mem::offset_of!(qjs::JSJitEntryHandle, pin)
            + std::mem::size_of::<*mut core::ffi::c_void>()
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitEntryHandle, helper_abi_version),
        std::mem::offset_of!(qjs::JSJitEntryHandle, stack_map_count) + 4
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitExecFrame, runtime_id),
        std::mem::offset_of!(qjs::JSJitExecFrame, runtime_api)
            + std::mem::size_of::<*const qjs::JSJitRuntimeAPI>()
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitExecFrame, frame_cookie),
        std::mem::offset_of!(qjs::JSJitExecFrame, runtime_id) + 8
    );
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitExecFrame, stack_capacity),
        std::mem::offset_of!(qjs::JSJitExecFrame, frame_cookie) + 8
    );
    assert_eq!(qjs::JS_JIT_HELPER_SCRATCH_SLOTS, 2);
    assert_eq!(
        std::mem::offset_of!(qjs::JSJitABIInfo, helper_table_fingerprint),
        std::mem::offset_of!(qjs::JSJitABIInfo, runtime_api_layout_fingerprint) + 8
    );
}

#[test]
#[cfg(feature = "test-support")]
fn backend_is_detached_before_runtime_drop() {
    let events = rquickjs_jit::test_support::record_lifecycle();
    {
        let _runtime = events.runtime();
    }
    assert_eq!(
        events.take(),
        ["attach", "detach", "backend_drop", "runtime_drop"]
    );
}

#[test]
#[cfg(feature = "test-support")]
fn cloned_runtime_outlives_the_detached_backend() {
    let events = rquickjs_jit::test_support::record_lifecycle();
    let runtime = events.runtime();
    let runtime_clone = runtime.runtime().clone();
    drop(runtime);

    assert_eq!(events.snapshot(), ["attach", "detach", "backend_drop"]);
    drop(runtime_clone);
    assert_eq!(
        events.take(),
        ["attach", "detach", "backend_drop", "runtime_drop"]
    );
}

#[test]
#[cfg(feature = "test-support")]
fn cloned_context_outlives_the_detached_backend() {
    let events = rquickjs_jit::test_support::record_lifecycle();
    let runtime = events.runtime();
    let context = rquickjs::Context::full(runtime.runtime()).unwrap();
    drop(runtime);

    assert_eq!(events.snapshot(), ["attach", "detach", "backend_drop"]);
    drop(context);
    assert_eq!(
        events.take(),
        ["attach", "detach", "backend_drop", "runtime_drop"]
    );
}

#[test]
#[cfg(feature = "test-support")]
fn duplicate_attachment_does_not_replace_the_first_backend() {
    assert!(rquickjs_jit::test_support::duplicate_attachment_is_rejected());
}

#[test]
#[cfg(feature = "test-support")]
fn every_abi_mismatch_is_rejected_before_backend_storage() {
    use rquickjs_jit::test_support::AbiMismatchFixture;

    for mismatch in AbiMismatchFixture::ALL {
        assert!(
            rquickjs_jit::test_support::mismatch_is_rejected_before_attach(mismatch),
            "fixture was accepted: {mismatch:?}"
        );
    }
}

#[test]
fn bundled_targets_share_jit_declarations() {
    let reference = bundled_binding(BUNDLED_TARGETS[0]);
    let reference = jit_declarations(&reference);
    for target in BUNDLED_TARGETS.iter().skip(1) {
        assert_eq!(
            jit_declarations(&bundled_binding(target)),
            reference,
            "{target}"
        );
    }
}

#[test]
#[cfg(all(feature = "test-support", feature = "bindgen"))]
fn bundled_targets_match_fresh_bindgen_output() {
    let generated = rquickjs_jit::test_support::fresh_bindgen_bindings()
        .expect("test must receive fresh bindgen output");
    let generated = jit_declarations(generated);
    for target in BUNDLED_TARGETS {
        assert_eq!(
            jit_declarations(&bundled_binding(target)),
            generated,
            "{target}"
        );
    }
}
