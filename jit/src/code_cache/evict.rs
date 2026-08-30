use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::Ordering, Arc},
};

use super::{ArtifactKey, CacheError, CachedArtifact};

/// Plans removals without mutating the cache. Selecting a Tier 2 artifact
/// releases its simulated Tier 1 deopt reference, so the next selection can
/// reconsider the newly eligible baseline.
pub(super) fn plan(
    artifacts: &BTreeMap<ArtifactKey, Arc<CachedArtifact>>,
    excluded: ArtifactKey,
    needed_bytes: usize,
    needed_code_bytes: usize,
    needed_metadata_bytes: usize,
) -> Result<Vec<ArtifactKey>, CacheError> {
    let mut deopt_references = artifacts
        .iter()
        .map(|(key, artifact)| (*key, artifact.deopt_references.load(Ordering::Acquire)))
        .collect::<BTreeMap<_, _>>();
    if let Some(target) = artifacts
        .get(&excluded)
        .and_then(|artifact| artifact.deopt_target_key())
    {
        release_deopt_reference(&mut deopt_references, target);
    }

    let mut selected = Vec::new();
    let mut selected_keys = BTreeSet::new();
    let mut freed_bytes = 0usize;
    let mut freed_code_bytes = 0usize;
    let mut freed_metadata_bytes = 0usize;
    while freed_bytes < needed_bytes
        || freed_code_bytes < needed_code_bytes
        || freed_metadata_bytes < needed_metadata_bytes
    {
        let candidate = artifacts
            .iter()
            .filter(|(key, artifact)| {
                **key != excluded
                    && !selected_keys.contains(*key)
                    && artifact.execution_pins.load(Ordering::Acquire) == 0
                    && deopt_references.get(key).copied().unwrap_or(0) == 0
            })
            .min_by_key(|(_, artifact)| artifact.eviction_plan_order())
            .map(|(key, artifact)| {
                (
                    *key,
                    artifact.charge_bytes,
                    artifact.code_bytes,
                    artifact.metadata_bytes,
                )
            });
        let Some((candidate, charge_bytes, code_bytes, metadata_bytes)) = candidate else {
            return Err(CacheError::AllArtifactsPinned);
        };
        freed_bytes = freed_bytes
            .checked_add(charge_bytes)
            .ok_or(CacheError::ChargeOverflow)?;
        freed_code_bytes = freed_code_bytes
            .checked_add(code_bytes)
            .ok_or(CacheError::ChargeOverflow)?;
        freed_metadata_bytes = freed_metadata_bytes
            .checked_add(metadata_bytes)
            .ok_or(CacheError::ChargeOverflow)?;
        selected.push(candidate);
        selected_keys.insert(candidate);
        if let Some(target) = artifacts
            .get(&candidate)
            .and_then(|artifact| artifact.deopt_target_key())
        {
            release_deopt_reference(&mut deopt_references, target);
        }
    }
    Ok(selected)
}

fn release_deopt_reference(
    deopt_references: &mut BTreeMap<ArtifactKey, usize>,
    target: ArtifactKey,
) {
    if let Some(references) = deopt_references.get_mut(&target) {
        *references = references.saturating_sub(1);
    }
}
