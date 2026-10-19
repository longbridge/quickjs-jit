// Vendored from LLRT d9fd873e793c (modules/llrt_dns/src/lib.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use crate::utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Ctx, Result,
};

use crate::dns::lookup::lookup;

mod lookup;

pub struct DnsModule;

impl ModuleDef for DnsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("lookup")?;

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("lookup", Func::from(lookup))?;
            Ok(())
        })?;

        Ok(())
    }
}

impl From<DnsModule> for ModuleInfo<DnsModule> {
    fn from(val: DnsModule) -> Self {
        ModuleInfo {
            name: "dns",
            module: val,
        }
    }
}
