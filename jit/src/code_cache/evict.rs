use std::{collections::BTreeMap, sync::Arc};

use super::{ArtifactKey, CachedArtifact};

pub(super) fn candidates(
    artifacts: &BTreeMap<ArtifactKey, Arc<CachedArtifact>>,
    excluded: ArtifactKey,
) -> Vec<(ArtifactKey, usize)> {
    let mut candidates = artifacts
        .iter()
        .filter(|(key, artifact)| **key != excluded && artifact.is_evictable())
        .map(|(key, artifact)| (artifact.eviction_order(), *key, artifact.charge_bytes))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(order, _, _)| *order);
    candidates
        .into_iter()
        .map(|(_, key, charge_bytes)| (key, charge_bytes))
        .collect()
}
