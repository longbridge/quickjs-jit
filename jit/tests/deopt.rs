use rquickjs_jit::ir::{
    DeoptMap, DeoptOwnership, DeoptPhase, Materialization, MaterializedValue, OptimizedFrameShape,
    TaggedValue,
};

#[test]
fn complete_deopt_map_materializes_every_slot_once() {
    let shape = OptimizedFrameShape::new(2, 3, 2);
    let map = DeoptMap::new(
        5,
        42,
        DeoptPhase::AfterEffect(9),
        vec![
            Materialization::argument(0, MaterializedValue::TaggedSlot(0)),
            Materialization::argument(1, MaterializedValue::Int32(7)),
            Materialization::local(0, MaterializedValue::Float64(-0.0)),
            Materialization::local(1, MaterializedValue::Undefined),
            Materialization::local(2, MaterializedValue::Null),
            Materialization::stack(0, MaterializedValue::Bool(true)),
            Materialization::stack(1, MaterializedValue::TaggedSlot(6)),
        ],
    );

    assert!(map.validate(shape).is_ok());
    let frame = map.materialize(shape).expect("complete map materializes");
    assert_eq!(frame.resume_pc(), 42);
    assert_eq!(frame.side_effect_epoch(), 9);
    assert_eq!(frame.slots().len(), 7);
    assert!(frame.slots()[2].is_negative_zero());
}

#[test]
fn malformed_deopt_maps_fail_closed_before_writing_any_slot() {
    let shape = OptimizedFrameShape::new(1, 1, 0);
    let duplicate = DeoptMap::new(
        1,
        8,
        DeoptPhase::BeforeEffect(3),
        vec![
            Materialization::argument(0, MaterializedValue::Int32(1)),
            Materialization::argument(0, MaterializedValue::Int32(2)),
        ],
    );
    let mut destination = vec![MaterializedValue::Poison; shape.slot_count()];
    assert!(duplicate.materialize_into(shape, &mut destination).is_err());
    assert_eq!(destination, vec![MaterializedValue::Poison; 2]);
}

#[test]
fn effect_phase_selects_exact_resume_boundary() {
    assert_eq!(DeoptPhase::BeforeEffect(4).side_effect_epoch(), 3);
    assert_eq!(DeoptPhase::AfterEffect(4).side_effect_epoch(), 4);
}

#[derive(Default)]
struct Refcounts {
    duplicated: Vec<u16>,
    released: Vec<u64>,
    fail_on: Option<u16>,
}

impl DeoptOwnership for Refcounts {
    type Error = ();
    fn duplicate(&mut self, source_slot: u16) -> Result<TaggedValue, Self::Error> {
        if self.fail_on == Some(source_slot) {
            return Err(());
        }
        self.duplicated.push(source_slot);
        Ok(TaggedValue::new(u64::from(source_slot), 7))
    }
    fn release(&mut self, value: TaggedValue) {
        self.released.push(value.payload);
    }
}

#[test]
fn owning_values_are_duplicated_before_publish_and_rolled_back_on_failure() {
    let shape = OptimizedFrameShape::new(1, 1, 0);
    let map = DeoptMap::new(
        1,
        6,
        DeoptPhase::AfterEffect(1),
        vec![
            Materialization::argument(0, MaterializedValue::TaggedSlot(3)),
            Materialization::local(0, MaterializedValue::TaggedSlot(4)),
        ],
    );
    let mut refs = Refcounts {
        fail_on: Some(4),
        ..Default::default()
    };
    assert!(map.materialize_owned(shape, &mut refs).is_err());
    assert_eq!(refs.duplicated, vec![3]);
    assert_eq!(refs.released, vec![3]);

    refs.fail_on = None;
    let frame = map.materialize_owned(shape, &mut refs).unwrap();
    assert_eq!(frame.owned_count(), 2);
}
