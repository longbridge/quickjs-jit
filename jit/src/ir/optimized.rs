use super::TaggedValue;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeoptSlot {
    Argument(u16),
    Local(u16),
    Stack(u16),
}

#[derive(Clone, Copy, Debug)]
pub enum MaterializedValue {
    Poison,
    Undefined,
    Null,
    Bool(bool),
    Int32(i32),
    Float64(f64),
    TaggedSlot(u16),
}

impl PartialEq for MaterializedValue {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Poison, Self::Poison)
            | (Self::Undefined, Self::Undefined)
            | (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int32(a), Self::Int32(b)) => a == b,
            (Self::Float64(a), Self::Float64(b)) => a.to_bits() == b.to_bits(),
            (Self::TaggedSlot(a), Self::TaggedSlot(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptimizedFrameShape {
    arguments: u16,
    locals: u16,
    stack: u16,
}

impl OptimizedFrameShape {
    pub const fn new(arguments: u16, locals: u16, stack: u16) -> Self {
        Self {
            arguments,
            locals,
            stack,
        }
    }
    pub const fn slot_count(self) -> usize {
        self.arguments as usize + self.locals as usize + self.stack as usize
    }
    fn index(self, slot: DeoptSlot) -> Option<usize> {
        match slot {
            DeoptSlot::Argument(i) if i < self.arguments => Some(i as usize),
            DeoptSlot::Local(i) if i < self.locals => Some(self.arguments as usize + i as usize),
            DeoptSlot::Stack(i) if i < self.stack => {
                Some(self.arguments as usize + self.locals as usize + i as usize)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeoptPhase {
    BeforeEffect(u64),
    AfterEffect(u64),
}

impl DeoptPhase {
    pub const fn side_effect_epoch(self) -> u64 {
        match self {
            Self::BeforeEffect(epoch) => epoch.saturating_sub(1),
            Self::AfterEffect(epoch) => epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Materialization {
    slot: DeoptSlot,
    value: MaterializedValue,
}

impl Materialization {
    pub const fn argument(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Argument(index),
            value,
        }
    }
    pub const fn local(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Local(index),
            value,
        }
    }
    pub const fn stack(index: u16, value: MaterializedValue) -> Self {
        Self {
            slot: DeoptSlot::Stack(index),
            value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeoptValidationError {
    SlotCount,
    DuplicateSlot,
    InvalidSlot,
    DestinationSize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeoptMap {
    guard: u32,
    resume_pc: u32,
    phase: DeoptPhase,
    slots: Box<[Materialization]>,
}

impl DeoptMap {
    pub fn new(guard: u32, resume_pc: u32, phase: DeoptPhase, slots: Vec<Materialization>) -> Self {
        Self {
            guard,
            resume_pc,
            phase,
            slots: slots.into_boxed_slice(),
        }
    }
    pub const fn guard(&self) -> u32 {
        self.guard
    }
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn phase(&self) -> DeoptPhase {
        self.phase
    }
    pub const fn materialization_count(&self) -> usize {
        self.slots.len()
    }
    pub fn validate(&self, shape: OptimizedFrameShape) -> Result<(), DeoptValidationError> {
        if self.slots.len() != shape.slot_count() {
            return Err(DeoptValidationError::SlotCount);
        }
        let mut seen = vec![false; shape.slot_count()];
        for recipe in &self.slots {
            let index = shape
                .index(recipe.slot)
                .ok_or(DeoptValidationError::InvalidSlot)?;
            if seen[index] {
                return Err(DeoptValidationError::DuplicateSlot);
            }
            seen[index] = true;
        }
        if seen.iter().any(|present| !present) {
            return Err(DeoptValidationError::SlotCount);
        }
        Ok(())
    }
    pub fn materialize(
        &self,
        shape: OptimizedFrameShape,
    ) -> Result<MaterializedFrame, DeoptValidationError> {
        let mut slots = vec![MaterializedValue::Poison; shape.slot_count()];
        self.materialize_into(shape, &mut slots)?;
        Ok(MaterializedFrame {
            resume_pc: self.resume_pc,
            side_effect_epoch: self.phase.side_effect_epoch(),
            slots: slots.into_boxed_slice(),
        })
    }
    pub fn materialize_into(
        &self,
        shape: OptimizedFrameShape,
        destination: &mut [MaterializedValue],
    ) -> Result<(), DeoptValidationError> {
        self.validate(shape)?;
        if destination.len() != shape.slot_count() {
            return Err(DeoptValidationError::DestinationSize);
        }
        let plan = self
            .slots
            .iter()
            .map(|recipe| (shape.index(recipe.slot).expect("validated"), recipe.value))
            .collect::<Vec<_>>();
        for (index, value) in plan {
            destination[index] = value;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedFrame {
    resume_pc: u32,
    side_effect_epoch: u64,
    slots: Box<[MaterializedValue]>,
}
impl MaterializedFrame {
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn side_effect_epoch(&self) -> u64 {
        self.side_effect_epoch
    }
    pub fn slots(&self) -> &[MaterializedValue] {
        &self.slots
    }
}

pub trait DeoptOwnership {
    type Error;
    fn duplicate(&mut self, source_slot: u16) -> Result<TaggedValue, Self::Error>;
    fn release(&mut self, value: TaggedValue);
}

#[derive(Debug)]
pub enum OwnedMaterializeError<E> {
    InvalidMap(DeoptValidationError),
    Ownership(E),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnedMaterializedValue {
    Scalar(MaterializedValue),
    Tagged(TaggedValue),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedMaterializedFrame {
    resume_pc: u32,
    side_effect_epoch: u64,
    slots: Box<[OwnedMaterializedValue]>,
    owned_count: usize,
}
impl OwnedMaterializedFrame {
    pub const fn resume_pc(&self) -> u32 {
        self.resume_pc
    }
    pub const fn side_effect_epoch(&self) -> u64 {
        self.side_effect_epoch
    }
    pub fn slots(&self) -> &[OwnedMaterializedValue] {
        &self.slots
    }
    pub const fn owned_count(&self) -> usize {
        self.owned_count
    }
}

impl DeoptMap {
    /// Executes the fallible ownership phase into private scratch storage. No
    /// caller-visible frame slot is changed until every duplication succeeds.
    pub fn materialize_owned<O: DeoptOwnership>(
        &self,
        shape: OptimizedFrameShape,
        ownership: &mut O,
    ) -> Result<OwnedMaterializedFrame, OwnedMaterializeError<O::Error>> {
        self.validate(shape)
            .map_err(OwnedMaterializeError::InvalidMap)?;
        let mut planned =
            vec![OwnedMaterializedValue::Scalar(MaterializedValue::Poison); shape.slot_count()];
        let mut owned = Vec::new();
        for recipe in &self.slots {
            let index = shape
                .index(recipe.slot)
                .expect("map validated before ownership");
            planned[index] = match recipe.value {
                MaterializedValue::TaggedSlot(source) => match ownership.duplicate(source) {
                    Ok(value) => {
                        owned.push(value);
                        OwnedMaterializedValue::Tagged(value)
                    }
                    Err(error) => {
                        for value in owned.drain(..).rev() {
                            ownership.release(value);
                        }
                        return Err(OwnedMaterializeError::Ownership(error));
                    }
                },
                scalar => OwnedMaterializedValue::Scalar(scalar),
            };
        }
        Ok(OwnedMaterializedFrame {
            resume_pc: self.resume_pc,
            side_effect_epoch: self.phase.side_effect_epoch(),
            slots: planned.into_boxed_slice(),
            owned_count: owned.len(),
        })
    }
}

impl MaterializedValue {
    pub fn is_negative_zero(self) -> bool {
        matches!(self, Self::Float64(value) if value.to_bits() == (-0.0f64).to_bits())
    }
}
