// Vendored from LLRT d9fd873e793c (libs/llrt_utils/src/clone.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{class::JsClass, Class, Ctx, Object, Result, Value};

pub trait StructuredClone<'js>: JsClass<'js> {
    fn structured_clone(&self, ctx: &Ctx<'js>) -> Result<Value<'js>>;
}

pub fn clone_platform_object<'js, T>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
) -> Result<Option<Value<'js>>>
where
    T: StructuredClone<'js>,
{
    if let Some(class) = Class::<T>::from_object(object) {
        return Ok(Some(class.borrow().structured_clone(ctx)?));
    }
    Ok(None)
}
