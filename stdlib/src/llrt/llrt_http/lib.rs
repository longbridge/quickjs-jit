// Vendored from LLRT d9fd873e793c (modules/llrt_http/src/lib.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use crate::utils::module::{export_default, ModuleInfo};
#[cfg(any(
    feature = "http-tls-ring",
    feature = "http-tls-aws-lc",
    feature = "http-tls-graviola",
    feature = "http-tls-openssl"
))]
use rquickjs::Class;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Result,
};

pub use self::config::*;

#[cfg(any(
    feature = "http-tls-ring",
    feature = "http-tls-aws-lc",
    feature = "http-tls-graviola",
    feature = "http-tls-openssl"
))]
mod client;
mod config;

#[cfg(any(
    feature = "http-tls-ring",
    feature = "http-tls-aws-lc",
    feature = "http-tls-graviola",
    feature = "http-tls-openssl"
))]
mod agent;

#[cfg(any(
    feature = "http-tls-ring",
    feature = "http-tls-aws-lc",
    feature = "http-tls-graviola",
    feature = "http-tls-openssl"
))]
pub use self::{agent::Agent, client::*};

pub struct HttpsModule;

impl ModuleDef for HttpsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        #[cfg(any(
            feature = "http-tls-ring",
            feature = "http-tls-aws-lc",
            feature = "http-tls-graviola",
            feature = "http-tls-openssl"
        ))]
        declare.declare(stringify!(Agent))?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            #[cfg(any(
                feature = "http-tls-ring",
                feature = "http-tls-aws-lc",
                feature = "http-tls-graviola",
                feature = "http-tls-openssl"
            ))]
            Class::<Agent>::define(default)?;

            let _ = default;
            Ok(())
        })
    }
}

impl From<HttpsModule> for ModuleInfo<HttpsModule> {
    fn from(val: HttpsModule) -> Self {
        ModuleInfo {
            name: "https",
            module: val,
        }
    }
}
