// Vendored from LLRT d9fd873e793c (libs/llrt_compression/src/streaming.rs); module paths and
// feature names adapted by scripts/import-stdlib.py.
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io::{self, Write};

#[cfg(all(not(any()), feature = "zlib"))]
use brotli as brotlic;

/// Streaming decompressor that maintains state across chunks
pub enum StreamingDecoder {
    #[cfg(any(any(), feature = "zlib"))]
    Gzip(flate2::write::GzDecoder<Vec<u8>>),
    #[cfg(any(any(), feature = "zlib"))]
    Deflate(flate2::write::ZlibDecoder<Vec<u8>>),
    #[cfg(any(any(), feature = "zlib-zstd"))]
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
    #[cfg(any(any(), feature = "zlib"))]
    Brotli(brotlic::DecompressorWriter<Vec<u8>>),
    Identity,
}

impl StreamingDecoder {
    pub fn new(encoding: &str) -> io::Result<Self> {
        match encoding {
            #[cfg(any(any(), feature = "zlib"))]
            "gzip" => Ok(Self::Gzip(flate2::write::GzDecoder::new(Vec::new()))),
            #[cfg(any(any(), feature = "zlib"))]
            "deflate" => Ok(Self::Deflate(flate2::write::ZlibDecoder::new(Vec::new()))),
            #[cfg(any(any(), feature = "zlib-zstd"))]
            "zstd" => Ok(Self::Zstd(zstd::stream::write::Decoder::new(Vec::new())?)),
            #[cfg(any())]
            "br" => Ok(Self::Brotli(brotlic::DecompressorWriter::new(Vec::new()))),
            #[cfg(all(not(any()), feature = "zlib"))]
            "br" => Ok(Self::Brotli(brotlic::DecompressorWriter::new(
                Vec::new(),
                8_096,
            ))),
            "" | "identity" => Ok(Self::Identity),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported encoding: {}", encoding),
            )),
        }
    }

    /// Decompress a chunk of data, returning the decompressed output
    pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            #[cfg(any(any(), feature = "zlib"))]
            Self::Gzip(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            #[cfg(any(any(), feature = "zlib"))]
            Self::Deflate(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            #[cfg(any(any(), feature = "zlib-zstd"))]
            Self::Zstd(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            #[cfg(any(any(), feature = "zlib"))]
            Self::Brotli(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
        }
    }

    /// Finish decompression and return any remaining data
    pub fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(Vec::new()),
            #[cfg(any(any(), feature = "zlib"))]
            Self::Gzip(decoder) => decoder.finish(),
            #[cfg(any(any(), feature = "zlib"))]
            Self::Deflate(decoder) => decoder.finish(),
            #[cfg(any(any(), feature = "zlib-zstd"))]
            Self::Zstd(decoder) => Ok(decoder.into_inner()),
            #[cfg(any())]
            Self::Brotli(decoder) => decoder
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
            #[cfg(all(not(any()), feature = "zlib"))]
            Self::Brotli(decoder) => decoder
                .into_inner()
                .map_err(|_| io::Error::other("brotli decompression failed")),
        }
    }
}
