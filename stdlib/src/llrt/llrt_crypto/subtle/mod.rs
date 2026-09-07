// Redistributed from LLRT; module paths and backend cfgs adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
mod crypto_key;
mod derive_algorithm;
mod derive_bits;
mod derive_keys;
mod digest;
mod encryption;
mod encryption_algorithm;
#[cfg(all())]
mod export_key;
mod generate_key;
#[cfg(all())]
mod import_key;
#[cfg(all())]
mod key_algorithm;
mod sign;
mod sign_algorithm;
mod util;
mod verify;
#[cfg(all())]
mod wrapping;

pub use crypto_key::CryptoKey;
pub use derive_bits::subtle_derive_bits;
pub use derive_keys::subtle_derive_key;
pub use digest::subtle_digest;
pub use encryption::subtle_decrypt;
pub use encryption::subtle_encrypt;
#[cfg(all())]
pub use export_key::subtle_export_key;
pub use generate_key::subtle_generate_key;
#[cfg(all())]
pub use import_key::subtle_import_key;
#[cfg(all())]
use key_algorithm::KeyAlgorithm;
pub use sign::subtle_sign;
pub use verify::subtle_verify;
#[cfg(all())]
pub use wrapping::subtle_unwrap_key;
#[cfg(all())]
pub use wrapping::subtle_wrap_key;

// Stub implementations for limited crypto providers (no _subtle-full)
#[cfg(not(all()))]
mod key_algorithm;
#[cfg(not(all()))]
use key_algorithm::KeyAlgorithm;

use crate::llrt_exceptions::DOMException;
use crate::llrt_utils::{object::ObjectExt, str_enum};
use rquickjs::{atom::PredefinedAtom, Ctx, Error, Exception, Object, Result, Value};

use crate::llrt_crypto::provider::{CryptoProvider, SimpleDigest};

use crate::llrt_crypto::hash::HashAlgorithm;

#[rquickjs::class]
#[derive(rquickjs::JsLifetime, rquickjs::class::Trace)]
pub struct SubtleCrypto {}

#[rquickjs::methods]
impl SubtleCrypto {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(prop, rename = PredefinedAtom::SymbolToStringTag, configurable)]
    pub fn to_string_tag() -> &'static str {
        stringify!(SubtleCrypto)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EllipticCurve {
    P256,
    P384,
    P521,
}

str_enum!(EllipticCurve,P256 => "P-256", P384 => "P-384", P521 => "P-521");

pub enum EncryptionMode {
    Encryption,
    #[allow(dead_code)]
    Wrapping(u8), //padding byte
}

pub fn rsa_hash_digest<'a>(
    ctx: &Ctx<'_>,
    key: &'a CryptoKey,
    data: &'a [u8],
    algorithm_name: &str,
) -> Result<(&'a HashAlgorithm, Vec<u8>)> {
    let hash = match &key.algorithm {
        KeyAlgorithm::Rsa { hash, .. } => hash,
        _ => return algorithm_mismatch_error(ctx, algorithm_name),
    };
    if !matches!(
        hash,
        HashAlgorithm::Sha1 | HashAlgorithm::Sha256 | HashAlgorithm::Sha384 | HashAlgorithm::Sha512
    ) {
        return Err(Exception::throw_message(
            ctx,
            "Only SHA-1, SHA-256, SHA-384 or SHA-512 is supported for RSA",
        ));
    }

    let mut hasher = crate::llrt_crypto::CRYPTO_PROVIDER.digest(*hash);
    hasher.update(data);
    let digest = hasher.finalize();

    Ok((hash, digest))
}

pub fn to_name_and_maybe_object<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> Result<(String, Result<Object<'js>>)> {
    let obj;
    let name = if let Some(string) = value.as_string() {
        obj = Err(Error::new_from_js_message(
            "string",
            "object",
            "algorithm is not an object",
        ));
        string.to_string()?
    } else if let Some(object) = value.into_object() {
        let name = object.get_required("name", "algorithm")?;
        obj = Ok(object);
        name
    } else {
        return Err(Exception::throw_message(
            ctx,
            "algorithm must be a string or an object",
        ));
    };
    Ok((name, obj))
}

pub fn normalize_algorithm_name(name: &str) -> String {
    let name = name.to_ascii_uppercase();
    match name.as_str() {
        "ED25519" => "Ed25519".to_string(),
        "RSASSA-PKCS1-V1_5" => "RSASSA-PKCS1-v1_5".to_string(),
        _ => name,
    }
}

pub fn algorithm_mismatch_error<T>(ctx: &Ctx<'_>, expected_algorithm: &str) -> Result<T> {
    Err(DOMException::type_mismatch_error(
        ctx,
        ["Key algorithm must be ", expected_algorithm].concat(),
    ))
}

pub fn algorithm_not_supported_error<T>(ctx: &Ctx<'_>) -> Result<T> {
    Err(DOMException::not_supported_error(
        ctx,
        "Algorithm not supported",
    ))
}

pub fn algorithm_invalid_access_error<T>(ctx: &Ctx<'_>, expected_algorithm: &str) -> Result<T> {
    Err(DOMException::invalid_access_error(
        ctx,
        ["Key algorithm must be ", expected_algorithm].concat(),
    ))
}

// Stub implementations for providers without _subtle-full
#[cfg(not(all()))]
mod stubs;
#[cfg(not(all()))]
pub use stubs::subtle_export_key;
#[cfg(not(all()))]
pub use stubs::subtle_import_key;
#[cfg(not(all()))]
pub use stubs::subtle_unwrap_key;
#[cfg(not(all()))]
pub use stubs::subtle_wrap_key;
