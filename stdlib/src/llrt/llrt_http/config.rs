// Vendored from LLRT d9fd873e793c (modules/llrt_http/src/config.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::Duration,
};

static CONNECTION_POOL_IDLE_TIMEOUT: AtomicU64 = AtomicU64::new(15);

pub fn set_pool_idle_timeout_seconds(seconds: u64) {
    CONNECTION_POOL_IDLE_TIMEOUT.store(seconds, Ordering::Relaxed);
}

pub fn get_pool_idle_timeout() -> Duration {
    Duration::from_secs(CONNECTION_POOL_IDLE_TIMEOUT.load(Ordering::Relaxed))
}

#[derive(Debug, Clone, Copy)]
pub enum HttpVersion {
    Http1_1,
    Http2,
}

static HTTP_VERSION: OnceLock<HttpVersion> = OnceLock::new();

pub fn set_http_version(version: HttpVersion) {
    _ = HTTP_VERSION.set(version);
}

pub fn get_http_version() -> HttpVersion {
    *HTTP_VERSION.get_or_init(|| {
        #[cfg(all(feature = "http-http2", feature = "http-http1"))]
        {
            HttpVersion::Http1_1
        }
        #[cfg(all(not(feature = "http-http1"), feature = "http-http2"))]
        {
            HttpVersion::Http2
        }
        #[cfg(all(not(feature = "http-http2"), feature = "http-http1"))]
        {
            HttpVersion::Http1_1
        }
        #[cfg(all(not(feature = "http-http2"), not(feature = "http-http1")))]
        {
            compile_error!("Either the `http1` or `http2` feature must be enabled")
        }
    })
}
