#![no_main]
use libfuzzer_sys::fuzz_target;
use rquickjs_jit::{
    code_cache::{Relocation, RelocationKind, RelocationTarget, ResolvedRelocation},
    platform::CodeAllocator,
};
fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }
    let offset = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let target = u64::from_le_bytes(data[4..12].try_into().unwrap());
    let addend = i32::from_le_bytes(data[12..16].try_into().unwrap()) as i64;
    let relocation = Relocation::new(offset, target, addend);
    let resolved = relocation.resolve_with(|symbol| match symbol {
        RelocationTarget::Absolute(value) => Some(*value),
        _ => None,
    });
    assert!(resolved.is_ok());
    if let Ok(allocator) = CodeAllocator::with_limit(4096) {
        if let Ok(mut writable) = allocator.allocate(64) {
            let kind = if data[0] & 1 == 0 {
                RelocationKind::Abs8
            } else {
                RelocationKind::X86PCRel4
            };
            let candidate = ResolvedRelocation::new(offset, kind, target, addend);
            let before = writable.bytes().to_vec();
            let result = writable.apply_relocations(&[candidate]);
            if result.is_err() {
                assert_eq!(
                    writable.bytes(),
                    before.as_slice(),
                    "invalid relocation changed mock publish bytes"
                );
            }
        }
    }
});
