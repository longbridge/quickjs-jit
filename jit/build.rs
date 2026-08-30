use std::{env, fs, path::PathBuf};

fn main() {
    const EXPECTED_COUNT: usize = 252;
    const EXPECTED_FINGERPRINT: u64 = 0x05d5_c086_7521_c077;
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let opcodes = rquickjs_sys::QJSJIT_GENERATED_OPCODES;
    assert_eq!(opcodes.len(), rquickjs_sys::QJSJIT_GENERATED_OPCODE_COUNT);
    assert_eq!(
        opcodes.len(),
        EXPECTED_COUNT,
        "QuickJS opcode count changed; audit and regenerate the closed Tier 1 policy"
    );
    assert_eq!(
        rquickjs_sys::QJSJIT_GENERATED_OPCODE_FINGERPRINT,
        EXPECTED_FINGERPRINT,
        "QuickJS opcode metadata changed; audit and regenerate the closed Tier 1 policy"
    );
    let mut generated = format!(
        "pub const GENERATED_OPCODE_COUNT: usize = {};\n\
         pub const GENERATED_OPCODE_FINGERPRINT: u64 = 0x{:016x};\n\
         pub static GENERATED_OPCODE_IDENTITIES: &[(u8, &str)] = &[\n",
        opcodes.len(),
        rquickjs_sys::QJSJIT_GENERATED_OPCODE_FINGERPRINT
    );
    for (expected_id, opcode) in opcodes.iter().enumerate() {
        assert_eq!(
            usize::from(opcode.opcode),
            expected_id,
            "opcode IDs are not dense"
        );
        generated.push_str(&format!("    ({}, {:?}),\n", opcode.opcode, opcode.name));
    }
    generated.push_str("];\n");
    fs::write(out.join("tier1-opcodes.rs"), generated).expect("write Tier 1 opcode identities");
}
