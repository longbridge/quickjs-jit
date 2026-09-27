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

/// TDZ checking mode for closure-variable stores. Values match the native
/// `JSJitVarRefMode` ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VarRefMode {
    Plain,
    Check,
    CheckInit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrOp {
    Poll {
        state: FrameStateId,
        kind: PollKind,
    },
    OsrLabel {
        state: FrameStateId,
    },
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
    Call {
        argc: u16,
        has_this: bool,
    },
    CallConstructor(u16),
    Regexp,
    /// `fclosure`/`fclosure8`: instantiate the constant-pool bytecode as a
    /// closure over the current frame's arguments, locals and var refs.
    FClosure(u32),
    /// `get_var_ref*`: load a closure variable, optionally with the TDZ check.
    GetVarRef {
        index: u16,
        checked: bool,
    },
    /// `put_var_ref*`/`set_var_ref*`: store a closure variable. `keep` leaves
    /// the stored value on the operand stack (`set_var_ref*`).
    PutVarRef {
        index: u16,
        mode: VarRefMode,
        keep: bool,
    },
    /// `close_loc`: detach the var ref capturing a lexical local.
    CloseLocal(u16),
    /// `set_name`: define the anonymous function/class name of the stack top.
    SetName(u32),
    GetArgument(u16),
    GetLocal(u16),
    GetLocalChecked(u16),
    GetLocalPair,
    PutArgument {
        index: u16,
        keep: bool,
    },
    PutLocal {
        index: u16,
        keep: bool,
    },
    PutLocalChecked {
        index: u16,
        initialize: bool,
    },
    SetLocalUninitialized(u16),
    Drop,
    Stack(StackOp),
    Unary(UnaryOp),
    PostUnary(UnaryOp),
    LocalUnary {
        index: u16,
        op: UnaryOp,
    },
    AddLocal(u16),
    Binary(BinaryOp),
    Jump(u32),
    Branch {
        target: u32,
        when_true: bool,
    },
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
