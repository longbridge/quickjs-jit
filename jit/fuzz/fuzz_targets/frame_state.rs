#![no_main]
use libfuzzer_sys::fuzz_target;
use rquickjs_jit::{
    code_cache::FrameState,
    ir::{DeoptMap, DeoptPhase, Materialization, MaterializedValue, OptimizedFrameShape},
};
fuzz_target!(|data: &[u8]| {
    let slots = data
        .chunks_exact(2)
        .take(1024)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    let state = FrameState::new(
        data.len() as u32,
        data.first().copied().unwrap_or(0) as u32,
        slots,
    );
    assert!(state.slots.len() <= 1024);
    let arguments = u16::from(data.first().copied().unwrap_or(0) % 8);
    let locals = u16::from(data.get(1).copied().unwrap_or(0) % 8);
    let stack = u16::from(data.get(2).copied().unwrap_or(0) % 8);
    let shape = OptimizedFrameShape::new(arguments, locals, stack);
    let mut recipes = Vec::new();
    for index in 0..shape.slot_count().min(64) {
        let value = MaterializedValue::TaggedSlot(
            data.get(index + 3).copied().unwrap_or(index as u8) as u16,
        );
        if index < arguments as usize {
            recipes.push(Materialization::argument(index as u16, value))
        } else if index < (arguments + locals) as usize {
            recipes.push(Materialization::local(index as u16 - arguments, value))
        } else {
            recipes.push(Materialization::stack(
                index as u16 - arguments - locals,
                value,
            ))
        }
    }
    if data.get(3).is_some_and(|byte| byte & 1 != 0) && !recipes.is_empty() {
        recipes.push(recipes[0]);
    }
    let map = DeoptMap::new(1, 2, DeoptPhase::BeforeEffect(1), recipes);
    let result = map.validate(shape);
    if result.is_ok() {
        assert_eq!(map.materialization_count(), shape.slot_count());
    }
});
