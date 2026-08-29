use crate::code_cache::{CacheError, CacheInsert, CodeCache, CompiledArtifact};

pub(super) fn publish(
    cache: &mut CodeCache,
    artifact: CompiledArtifact,
) -> Result<CacheInsert, CacheError> {
    cache.insert(artifact)
}
