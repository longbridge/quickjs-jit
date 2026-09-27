//! Baseline operations. Every operation is explicitly tagged with its source PC.

use super::FrameStateId;

/// A raw 16-byte QuickJS value split into its payload and tag words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaggedValue {
    pub payload: u64,
    pub tag: i64,
}

impl TaggedValue {
    pub const fn new(payload: u64, tag: i64) -> Self {
        Self { payload, tag }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackOp {
    Nip,
    Nip1,
    Dup,
    Dup1,
    Dup2,
    Dup3,
    Insert2,
    Insert3,
    Insert4,
    Perm3,
    Perm4,
    Perm5,
    Swap,
    Swap2,
    Rot3Left,
    Rot3Right,
    Rot4Left,
    Rot5Left,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    IsUndefinedOrNull,
    IsUndefined,
    IsNull,
    Plus,
    Neg,
    Increment,
    Decrement,
    BitNot,
    LogicalNot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    ShiftRightUnsigned,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
}

/// Opcodes executed with exact interpreter semantics by the `GENERIC_OP`
/// helper (or `BINARY_ARITH_SLOW` for `pow`). Atoms and 8-bit immediates are
/// the verified bytecode operands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenericOp {
    /// `push_this`: pushes the (possibly boxed) receiver.
    PushThis,
    /// `special_object`: pushes the frame object selected by the immediate.
    SpecialObject(u8),
    /// `get_var_undef`: pushes a global or `undefined` when it is missing.
    GetVarUndef(u32),
    /// `delete_var`: pushes the boolean result of deleting a global binding.
    DeleteVar(u32),
    /// `put_var`: pops a value into a global binding.
    PutVar(u32),
    /// `typeof`: replaces the top value with its type string.
    TypeOf,
    /// `typeof_is_undefined`: replaces the top value with a boolean.
    TypeOfIsUndefined,
    /// `typeof_is_function`: replaces the top value with a boolean.
    TypeOfIsFunction,
    /// `to_object`: converts the top value to an object in place.
    ToObject,
    /// `to_propkey2`: checks that `sp[-2]` is object-coercible and converts
    /// the key at `sp[-1]` to a property key in place.
    ToPropertyKey2,
    /// `in`: `key in object`, producing a boolean.
    In,
    /// `instanceof`: `value instanceof constructor`, producing a boolean.
    InstanceOf,
    /// `delete`: `delete object[key]`, producing a boolean.
    Delete,
    /// `pow`: exponentiation through the exact arithmetic slow path.
    Pow,
}

impl GenericOp {
    /// Values consumed from the operand stack and values pushed back.
    pub const fn stack_effect(self) -> (usize, usize) {
        match self {
            Self::PushThis | Self::SpecialObject(_) | Self::GetVarUndef(_) | Self::DeleteVar(_) => {
                (0, 1)
            }
            Self::PutVar(_) => (1, 0),
            Self::TypeOf | Self::TypeOfIsUndefined | Self::TypeOfIsFunction | Self::ToObject => {
                (1, 1)
            }
            Self::ToPropertyKey2 => (2, 2),
            Self::In | Self::InstanceOf | Self::Delete | Self::Pow => (2, 1),
        }
    }

    /// Whether the single result is always a JS boolean.
    pub const fn produces_boolean(self) -> bool {
        matches!(
            self,
            Self::TypeOfIsUndefined
                | Self::TypeOfIsFunction
                | Self::In
                | Self::InstanceOf
                | Self::Delete
                | Self::DeleteVar(_)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrOp {
    Poll { state: FrameStateId, kind: PollKind },
    OsrLabel { state: FrameStateId },
    Nop,
    Push(TaggedValue),
    ResolveConstant(u32),
    ResolveAtom(u32),
    GetGlobal(u32),
    NewObject,
    NewArrayFrom(u16),
    GetProperty(u32),
    GetPropertyKeep(u32),
    SetProperty(u32),
    DefineProperty(u32),
    GetElement,
    SetElement,
    DefineElement,
    ToPropertyKey,
    Call { argc: u16, has_this: bool },
    CallConstructor(u16),
    Regexp,
    Generic(GenericOp),
    GetArgument(u16),
    GetLocal(u16),
    GetLocalChecked(u16),
    GetLocalPair,
    PutArgument { index: u16, keep: bool },
    PutLocal { index: u16, keep: bool },
    PutLocalChecked { index: u16, initialize: bool },
    SetLocalUninitialized(u16),
    Drop,
    Stack(StackOp),
    Unary(UnaryOp),
    PostUnary(UnaryOp),
    LocalUnary { index: u16, op: UnaryOp },
    AddLocal(u16),
    Binary(BinaryOp),
    Jump(u32),
    Branch { target: u32, when_true: bool },
    Return,
    ReturnUndefined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollKind {
    Entry,
    Periodic,
    LoopHeader,
    Return,
    Edge,
}
