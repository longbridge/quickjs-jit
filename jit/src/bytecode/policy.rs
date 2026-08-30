//! Closed Tier 1 policy for the authoritative, build-generated opcode set.

include!(concat!(env!("OUT_DIR"), "/tier1-opcodes.rs"));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelperId {
    Dup,
    Free,
    ResolveConst,
    ToNumeric,
    ToBool,
    AddSlow,
    CompareSlow,
    GetProperty,
    SetProperty,
    Call,
    NewArray,
    NewObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackReason {
    UnsupportedOpcode,
    DirectEval,
    WithScope,
    Generator,
    Async,
    DynamicImport,
    ExplicitResourceManagement,
    ExceptionRegion,
    ClosureFrame,
    ExtendedFrame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier1Policy {
    Native,
    Helper(HelperId),
    Reject(FallbackReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tier1Rejection {
    pc: u32,
    reason: FallbackReason,
}

impl Tier1Rejection {
    pub(crate) const fn new(pc: u32, reason: FallbackReason) -> Self {
        Self { pc, reason }
    }

    pub const fn pc(self) -> u32 {
        self.pc
    }

    pub const fn reason(self) -> FallbackReason {
        self.reason
    }
}

pub fn tier1_policy(id: u8) -> Option<Tier1Policy> {
    let (_, name) = *GENERATED_OPCODE_IDENTITIES.get(id as usize)?;
    Some(policy_by_name(name))
}

fn policy_by_name(name: &str) -> Tier1Policy {
    use FallbackReason as R;
    use HelperId as H;
    use Tier1Policy as P;
    match name {
        "eval" | "apply_eval" => P::Reject(R::DirectEval),
        "with_get_var" | "with_put_var" | "with_delete_var" | "with_make_ref" | "with_get_ref"
        | "with_get_ref_undef" => P::Reject(R::WithScope),
        "initial_yield" | "yield" | "yield_star" => P::Reject(R::Generator),
        "return_async"
        | "for_await_of_start"
        | "async_yield_star"
        | "await"
        | "using_dispose_async" => P::Reject(R::Async),
        "import" => P::Reject(R::DynamicImport),
        "using_dispose" | "using_dispose_init" => P::Reject(R::ExplicitResourceManagement),
        "catch" | "gosub" | "ret" | "nip_catch" => P::Reject(R::ExceptionRegion),
        name if name.contains("var_ref") => P::Reject(R::ClosureFrame),
        "push_this"
        | "push_new_target"
        | "push_home_object"
        | "push_special_object"
        | "set_name" => P::Reject(R::ExtendedFrame),

        "nop" | "undefined" | "push_undefined" | "null" | "push_null" | "push_false"
        | "push_true" | "push_minus1" | "push_0" | "push_1" | "push_2" | "push_3" | "push_4"
        | "push_5" | "push_6" | "push_7" | "push_i8" | "push_i16" | "push_i32" | "goto"
        | "goto8" | "goto16" | "return" | "return_undef" => P::Native,
        "push_const" | "push_const8" => P::Helper(H::ResolveConst),
        "object" => P::Helper(H::NewObject),
        "array_from" => P::Helper(H::NewArray),
        "get_field" => P::Helper(H::GetProperty),
        "put_field" => P::Helper(H::SetProperty),
        "call" | "call0" | "call1" | "call2" | "call3" | "call_method" => P::Helper(H::Call),
        "get_arg" | "get_arg0" | "get_arg1" | "get_arg2" | "get_arg3" | "get_loc" | "get_loc8"
        | "get_loc0" | "get_loc1" | "get_loc2" | "get_loc3" | "get_loc_check" | "get_loc0_loc1"
        | "dup" | "dup1" | "dup2" | "dup3" => P::Helper(H::Dup),
        "put_arg"
        | "put_arg0"
        | "put_arg1"
        | "put_arg2"
        | "put_arg3"
        | "set_arg"
        | "set_arg0"
        | "set_arg1"
        | "set_arg2"
        | "set_arg3"
        | "put_loc"
        | "put_loc8"
        | "put_loc0"
        | "put_loc1"
        | "put_loc2"
        | "put_loc3"
        | "set_loc"
        | "set_loc8"
        | "set_loc0"
        | "set_loc1"
        | "set_loc2"
        | "set_loc3"
        | "put_loc_check"
        | "put_loc_check_init"
        | "set_loc_uninitialized"
        | "drop"
        | "nip"
        | "nip1"
        | "insert2"
        | "insert3"
        | "insert4"
        | "perm3"
        | "perm4"
        | "perm5"
        | "swap"
        | "swap2"
        | "rot3l"
        | "rot3r"
        | "rot4l"
        | "rot5l" => P::Helper(H::Free),
        "plus" | "neg" | "inc" | "dec" | "post_inc" | "post_dec" | "inc_loc" | "dec_loc"
        | "add_loc" | "not" => P::Helper(H::ToNumeric),
        "lnot" | "if_true" | "if_true8" | "if_false" | "if_false8" => P::Helper(H::ToBool),
        "add" => P::Helper(H::AddSlow),
        "lt" | "lte" | "gt" | "gte" | "eq" | "neq" | "strict_eq" | "strict_neq" => {
            P::Helper(H::CompareSlow)
        }
        "sub" | "mul" | "div" | "mod" | "and" | "or" | "xor" | "shl" | "sar" | "shr" => {
            P::Helper(H::ToNumeric)
        }
        _ => P::Reject(R::UnsupportedOpcode),
    }
}
