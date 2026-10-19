// Vendored from LLRT d9fd873e793c (modules/llrt_navigator/src/lib.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Ctx, Object, Result};

fn get_user_agent() -> &'static str {
    concat!("llrt ", "0.9.0-beta")
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();

    let navigator = Object::new(ctx.clone())?;

    navigator.set("userAgent", get_user_agent())?;

    globals.set("navigator", navigator)?;

    Ok(())
}
