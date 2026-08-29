use std::{collections::BTreeMap, sync::Arc};

use super::{ArtifactKey, CachedArtifact};

pub(super) fn candidate(
    artifacts: &BTreeMap<ArtifactKey, Arc<CachedArtifact>>,
) -> Option<ArtifactKey> {
    artifacts
        .iter()
        .filter(|(_, artifact)| artifact.is_evictable())
        .min_by_key(|(_, artifact)| artifact.eviction_order())
        .map(|(key, _)| *key)
}
