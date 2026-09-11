//! Bounded, caller-context construction of effect-free inline regions.
//!
//! The first admission policy follows only statically selected forward paths.
//! Unknown branches, mutation and runtime operations retain the ordinary call.

use super::ScalarBinaryOp;
use crate::{
    bytecode::VerifiedFunction,
    code_cache::ArtifactKey,
    runtime::{CallSpecializationKey, FeedbackRepresentation},
};

#[derive(Clone, Debug)]
pub struct InlineCallee {
    pub artifact: ArtifactKey,
    pub call: CallSpecializationKey,
    pub body: VerifiedFunction,
}

#[derive(Clone, Debug)]
pub(super) enum InlineValue {
    Argument(usize),
    Int32(i32),
    Bool(bool),
    Binary {
        op: ScalarBinaryOp,
        lhs: usize,
        rhs: usize,
        pc: u32,
        stack: Box<[usize]>,
    },
}

#[derive(Debug)]
pub(super) struct InlinePlan {
    pub values: Vec<InlineValue>,
    pub result: usize,
}

impl InlineCallee {
    pub(super) fn plan(&self, argument_bool: impl Fn(usize) -> Option<bool>) -> Option<InlinePlan> {
        let snapshot = self.body.snapshot();
        if self.call.callee().id != snapshot.function_id()
            || self.call.callee().generation != snapshot.generation()
            || self.artifact.function_id != snapshot.function_id()
            || self.artifact.generation != snapshot.generation()
            || self.artifact.source_revision != snapshot.source_revision()
            || self.artifact.opcode_fingerprint != snapshot.opcode_fingerprint()
            || self.call.callee_identity() == 0
            || self.call.callee_bytecode_identity() == 0
            || self.call.arguments().len() != usize::from(snapshot.arg_count())
            || self.call.result() != FeedbackRepresentation::Int32
            || self.call.arguments().iter().any(|r| {
                !matches!(
                    r,
                    FeedbackRepresentation::Int32 | FeedbackRepresentation::Bool
                )
            })
            || snapshot.local_count() != 0
            || snapshot.closure_count() != 0
            || !snapshot.exception_map().is_empty()
            || self.body.instructions().len() > 128
        {
            return None;
        }
        let mut values = (0..self.call.arguments().len())
            .map(InlineValue::Argument)
            .collect::<Vec<_>>();
        let mut stack = Vec::new();
        let instructions = self.body.instructions();
        let mut cursor = 0;
        for _ in 0..128 {
            let instruction = instructions.get(cursor)?;
            let name = instruction.opcode().name();
            let mut push = |value| {
                let id = values.len();
                values.push(value);
                stack.push(id);
            };
            if let Some(value) = super::scalar::integer_constant(name, instruction.bytes()) {
                push(InlineValue::Int32(value));
            } else if matches!(name, "push_true" | "push_false") {
                push(InlineValue::Bool(name == "push_true"));
            } else if name.starts_with("get_arg") {
                let index = usize::from(super::optimized::indexed_node_operand(
                    name,
                    instruction.bytes(),
                )?);
                if index >= self.call.arguments().len() {
                    return None;
                }
                stack.push(index);
            } else {
                match name {
                    "add" | "sub" => {
                        let state = stack.clone().into_boxed_slice();
                        let rhs = stack.pop()?;
                        let lhs = stack.pop()?;
                        let numeric = |id: usize| match values.get(id) {
                            Some(InlineValue::Argument(index)) => {
                                self.call.arguments()[*index] == FeedbackRepresentation::Int32
                            }
                            Some(InlineValue::Int32(_) | InlineValue::Binary { .. }) => true,
                            _ => false,
                        };
                        if !numeric(lhs) || !numeric(rhs) {
                            return None;
                        }
                        let id = values.len();
                        values.push(InlineValue::Binary {
                            op: if name == "add" {
                                ScalarBinaryOp::Add
                            } else {
                                ScalarBinaryOp::Sub
                            },
                            lhs,
                            rhs,
                            pc: instruction.pc(),
                            stack: state,
                        });
                        stack.push(id);
                    }
                    "if_false" | "if_false8" | "if_true" | "if_true8" => {
                        let condition = stack.pop()?;
                        let truth = match values.get(condition)? {
                            InlineValue::Bool(value) => *value,
                            InlineValue::Int32(value) => *value != 0,
                            InlineValue::Argument(index) => argument_bool(*index)?,
                            _ => return None,
                        };
                        if truth == name.starts_with("if_true") {
                            let target = u32::try_from(instruction.branch_target()?).ok()?;
                            if target <= instruction.pc() {
                                return None;
                            }
                            cursor = instructions
                                .binary_search_by_key(&target, |i| i.pc())
                                .ok()?;
                            continue;
                        }
                    }
                    "goto" | "goto8" | "goto16" => {
                        let target = u32::try_from(instruction.branch_target()?).ok()?;
                        if target <= instruction.pc() {
                            return None;
                        }
                        cursor = instructions
                            .binary_search_by_key(&target, |i| i.pc())
                            .ok()?;
                        continue;
                    }
                    "drop" => {
                        stack.pop()?;
                    }
                    "nop" => {}
                    "return" => {
                        let result = stack.pop()?;
                        if !stack.is_empty() {
                            return None;
                        }
                        match values.get(result)? {
                            InlineValue::Bool(_) => return None,
                            InlineValue::Argument(index)
                                if self.call.arguments()[*index]
                                    != FeedbackRepresentation::Int32 =>
                            {
                                return None
                            }
                            _ => return Some(InlinePlan { values, result }),
                        }
                    }
                    _ => return None,
                }
            }
            if stack.len() > 16 {
                return None;
            }
            cursor += 1;
        }
        None
    }
}
