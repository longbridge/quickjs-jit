#![cfg(all(
    feature = "compiler",
    feature = "test-support",
    target_os = "windows",
    target_arch = "aarch64",
    target_endian = "little"
))]

use rquickjs_jit::{
    bytecode::opcode,
    compiler::baseline::BaselineCompiler,
    platform::CodeMemoryError,
    test_support::{compile_implemented_fixture, verified_bytecode},
};

#[test]
fn compilation_succeeds_and_publication_explicitly_falls_back_without_native_unwind_support() {
    let function = verified_bytecode(vec![opcode::RETURN_UNDEF], 0, 0);
    let code = compile_implemented_fixture(&BaselineCompiler::host(), &function)
        .expect("Windows ARM64 Tier 1 compilation succeeds");

    assert!(matches!(
        code.publish(),
        Err(CodeMemoryError::UnwindRegistrationUnsupported)
    ));
}
