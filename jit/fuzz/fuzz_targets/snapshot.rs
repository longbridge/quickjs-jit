#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let first = rquickjs_jit::bytecode::decode_raw(data);
    let second = rquickjs_jit::bytecode::decode_raw(data);
    assert_eq!(first, second);
    if let Ok(instructions) = first {
        let mut cursor = 0usize;
        for instruction in instructions {
            assert_eq!(instruction.pc() as usize, cursor);
            assert_eq!(instruction.bytes().len(), instruction.size());
            cursor = cursor.checked_add(instruction.size()).unwrap();
            assert!(cursor <= data.len());
        }
        assert_eq!(cursor, data.len());
    }
});
