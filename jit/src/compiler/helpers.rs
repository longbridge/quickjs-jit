use rquickjs_core::qjs;

use super::CompileFailure;

#[derive(Clone, Copy, Debug)]
pub(super) struct FrameLayout {
    pub arg_buf: i32,
    pub var_buf: i32,
    pub stack_base: i32,
    pub stack_top: i32,
    pub bytecode_start: i32,
    pub result: i32,
    pub runtime_api: i32,
    pub poll: i32,
    pub value_tag: i32,
}

impl FrameLayout {
    pub fn validated(pointer_bytes: u8) -> Result<Self, CompileFailure> {
        if pointer_bytes != 8
            || core::mem::size_of::<qjs::JSValue>() != 16
            || core::mem::align_of::<qjs::JSValue>() != 8
            || core::mem::size_of::<qjs::JSJitExit>() != 24
        {
            return Err(CompileFailure::InvalidArtifact);
        }
        let offset =
            |value: usize| i32::try_from(value).map_err(|_| CompileFailure::InvalidArtifact);
        Ok(Self {
            arg_buf: offset(core::mem::offset_of!(qjs::JSJitExecFrame, arg_buf))?,
            var_buf: offset(core::mem::offset_of!(qjs::JSJitExecFrame, var_buf))?,
            stack_base: offset(core::mem::offset_of!(qjs::JSJitExecFrame, stack_base))?,
            stack_top: offset(core::mem::offset_of!(qjs::JSJitExecFrame, stack_top))?,
            bytecode_start: offset(core::mem::offset_of!(qjs::JSJitExecFrame, bytecode_start))?,
            result: offset(core::mem::offset_of!(qjs::JSJitExecFrame, result))?,
            runtime_api: offset(core::mem::offset_of!(qjs::JSJitExecFrame, runtime_api))?,
            poll: offset(core::mem::offset_of!(qjs::JSJitRuntimeAPI, interrupt_poll))?,
            value_tag: offset(core::mem::offset_of!(qjs::JSValue, tag))?,
        })
    }
}
