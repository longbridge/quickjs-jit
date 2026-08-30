use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
};

use rquickjs::{Context, Function, Runtime};
use rquickjs_core::{
    qjs,
    runtime::{JitBackend, JitFunctionRegistry},
};
use rquickjs_jit::{
    abi::JitExitExt,
    bytecode::{opcode, CompileSnapshot, VerifiedFunction, VerifyLimits},
    code_cache::{
        ArtifactDependency, ArtifactKey, CodeAllocation, CodeCache, CompiledArtifact, FrameState,
        Relocation, StackMap,
    },
    runtime::{
        ArtifactEnvironment, CompileCompletion, CompileFailure, CompileState, CompletionSendError,
        Coordinator, FunctionKey, QueueError, Tier,
    },
};

fn coordinator_snapshot() -> VerifiedFunction {
    CompileSnapshot::from_untrusted_bytecode(vec![opcode::RETURN_UNDEF], 0, 0, 0, 0)
        .verify(VerifyLimits::default())
        .expect("minimal coordinator snapshot verifies")
}

fn coordinator(max_attempts: u8) -> Coordinator {
    Coordinator::with_limits(8, 8, max_attempts, 3)
}

#[test]
fn stale_result_is_never_installed() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(7, 3);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().expect("queued request");

    coordinator.retire(key);
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: request.artifact_key(),
        attempt_id: request.attempt_id(),
        result: Ok(CompiledArtifact::empty(request.artifact_key())),
    });

    assert_eq!(coordinator.state(key), CompileState::Retired);
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().installed, 0);
}

#[test]
fn repeated_failures_blacklist_only_the_generation() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(9, 1);

    for attempt in 1..=4 {
        coordinator
            .queue(key, Tier::Baseline, coordinator_snapshot())
            .unwrap();
        let request = coordinator.begin_next().expect("queued attempt begins");
        coordinator.complete(CompileCompletion {
            key,
            requested_tier: Tier::Baseline,
            artifact_key: request.artifact_key(),
            attempt_id: request.attempt_id(),
            result: Err(CompileFailure::UnsupportedOpcode),
        });
        if attempt < 4 {
            let CompileState::Backoff { retry_after, .. } = coordinator.state(key) else {
                panic!("failure before the limit must enter backoff")
            };
            coordinator.advance_clock(retry_after);
        }
    }

    assert_eq!(coordinator.state(key), CompileState::Blacklisted);
    assert_eq!(
        coordinator.state(FunctionKey::new(9, 2)),
        CompileState::Cold
    );
}

fn artifact_key(function_id: u64, generation: u64, tier: Tier) -> ArtifactKey {
    ArtifactKey {
        runtime_id: 1,
        function_id,
        generation,
        tier,
        target_isa: 2,
        cpu_features: 3,
        abi_fingerprint: 4,
        source_revision: 5,
        opcode_fingerprint: 6,
        config_fingerprint: 7,
    }
}

fn artifact_with_code(key: ArtifactKey, bytes: usize) -> CompiledArtifact {
    CompiledArtifact::from_parts(
        key,
        CodeAllocation::inert(vec![0; bytes]),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
}

#[test]
fn artifact_identity_distinguishes_every_compatibility_field() {
    let base = artifact_key(11, 12, Tier::Baseline);
    let variants = [
        ArtifactKey {
            runtime_id: 99,
            ..base
        },
        ArtifactKey {
            function_id: 99,
            ..base
        },
        ArtifactKey {
            generation: 99,
            ..base
        },
        ArtifactKey {
            tier: Tier::Optimizing,
            ..base
        },
        ArtifactKey {
            target_isa: 99,
            ..base
        },
        ArtifactKey {
            cpu_features: 99,
            ..base
        },
        ArtifactKey {
            abi_fingerprint: 99,
            ..base
        },
        ArtifactKey {
            source_revision: 99,
            ..base
        },
        ArtifactKey {
            opcode_fingerprint: 99,
            ..base
        },
        ArtifactKey {
            config_fingerprint: 99,
            ..base
        },
    ];

    let identities = std::iter::once(base)
        .chain(variants)
        .collect::<HashSet<_>>();
    assert_eq!(identities.len(), 11);
}

#[test]
fn cache_evicts_low_benefit_least_recent_artifact() {
    let mut cache = CodeCache::new(3);
    let a = artifact_key(1, 1, Tier::Baseline);
    let b = artifact_key(2, 1, Tier::Baseline);
    let c = artifact_key(3, 1, Tier::Baseline);
    let d = artifact_key(4, 1, Tier::Baseline);
    for key in [a, b, c] {
        cache.insert(artifact_with_code(key, 1)).unwrap();
    }
    cache.record_benefit(a, 10).unwrap();
    assert!(cache.touch(b));

    assert_eq!(
        cache.insert(artifact_with_code(d, 1)).unwrap().evicted(),
        Some(c)
    );
    assert!(cache.contains(a));
    assert!(cache.contains(b));
    assert!(!cache.contains(c));
    assert!(cache.contains(d));
}

#[test]
fn active_execution_pin_prevents_eviction() {
    let mut cache = CodeCache::new(3);
    let a = artifact_key(1, 1, Tier::Baseline);
    let b = artifact_key(2, 1, Tier::Baseline);
    let c = artifact_key(3, 1, Tier::Baseline);
    let d = artifact_key(4, 1, Tier::Baseline);
    for key in [a, b, c] {
        cache.insert(artifact_with_code(key, 1)).unwrap();
    }
    let pin = cache.pin(a).expect("installed artifact can be pinned");

    cache.insert(artifact_with_code(d, 1)).unwrap();
    assert!(cache.contains(a));
    assert_eq!(pin.key(), a);
}

#[test]
fn cache_enforces_byte_quota_with_deterministic_multi_eviction() {
    let mut cache = CodeCache::new(8);
    let a = artifact_key(1, 1, Tier::Baseline);
    let b = artifact_key(2, 1, Tier::Baseline);
    let c = artifact_key(3, 1, Tier::Baseline);
    cache.insert(artifact_with_code(a, 4)).unwrap();
    cache.insert(artifact_with_code(b, 4)).unwrap();

    let insertion = cache.insert(artifact_with_code(c, 8)).unwrap();

    assert_eq!(insertion.evictions(), &[a, b]);
    assert_eq!(cache.charged_bytes(), 8);
    assert!(!cache.contains(a));
    assert!(!cache.contains(b));
    assert!(cache.contains(c));
}

#[test]
fn cache_rejects_single_oversize_artifact_without_mutation() {
    let mut cache = CodeCache::new(3);
    let key = artifact_key(1, 1, Tier::Baseline);

    assert_eq!(
        cache.insert(artifact_with_code(key, 4)).unwrap_err(),
        rquickjs_jit::code_cache::CacheError::ArtifactTooLarge
    );
    assert_eq!(cache.charged_bytes(), 0);
    assert!(cache.is_empty());
}

#[test]
fn cache_charges_owned_relocation_metadata() {
    let mut cache = CodeCache::new(std::mem::size_of::<Relocation>() - 1);
    let key = artifact_key(1, 1, Tier::Baseline);
    let artifact = CompiledArtifact::from_parts(
        key,
        CodeAllocation::inert(Vec::new()),
        vec![Relocation::new(0, 0, 0)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(
        cache.insert(artifact).unwrap_err(),
        rquickjs_jit::code_cache::CacheError::ArtifactTooLarge
    );
}

#[test]
fn replacement_and_invalidation_update_cache_byte_charge() {
    let mut cache = CodeCache::new(5);
    let replaced = artifact_key(1, 1, Tier::Baseline);
    let invalidated = artifact_key(2, 1, Tier::Baseline);
    cache.insert(artifact_with_code(replaced, 3)).unwrap();
    cache.insert(artifact_with_code(invalidated, 2)).unwrap();
    assert_eq!(cache.charged_bytes(), 5);

    cache.insert(artifact_with_code(replaced, 1)).unwrap();
    assert_eq!(cache.charged_bytes(), 3);
    assert_eq!(cache.invalidate(FunctionKey::new(2, 1)), 1);
    assert_eq!(cache.charged_bytes(), 1);
}

#[test]
fn optimizing_artifact_requires_exact_baseline_deopt_target() {
    let mut cache = CodeCache::new(3);
    let optimizing = artifact_key(1, 1, Tier::Optimizing);

    assert!(matches!(
        cache.insert(CompiledArtifact::empty(optimizing)),
        Err(rquickjs_jit::code_cache::CacheError::MissingDeoptTarget)
    ));
    assert!(cache.is_empty());
}

#[test]
fn active_optimizing_code_retains_baseline_after_invalidation() {
    let mut cache = CodeCache::new(3);
    let function = FunctionKey::new(1, 1);
    let baseline = artifact_key(function.id, function.generation, Tier::Baseline);
    let optimizing = baseline.with_tier(Tier::Optimizing);
    cache.insert(CompiledArtifact::empty(baseline)).unwrap();
    cache.insert(CompiledArtifact::empty(optimizing)).unwrap();
    assert_eq!(cache.deopt_references(baseline), Some(1));
    let optimizing_pin = cache.pin(optimizing).unwrap();

    cache.invalidate(function);
    assert!(cache.contains(optimizing));
    assert!(cache.contains(baseline));
    assert_eq!(cache.deopt_references(baseline), Some(1));

    drop(optimizing_pin);
    assert_eq!(cache.collect_invalidated(), 2);
    assert!(!cache.contains(optimizing));
    assert!(!cache.contains(baseline));
}

#[test]
fn bounded_reclamation_revisits_tier1_after_removing_tier2() {
    let mut cache = CodeCache::new(2);
    let function = FunctionKey::new(1, 1);
    let baseline = artifact_key(function.id, function.generation, Tier::Baseline);
    let optimizing = baseline.with_tier(Tier::Optimizing);
    cache.insert(artifact_with_code(baseline, 1)).unwrap();
    cache.insert(artifact_with_code(optimizing, 1)).unwrap();
    let pin = cache.pin(optimizing).unwrap();
    assert_eq!(cache.invalidate(function), 0);
    drop(pin);

    let first = cache.poll_reclamation_with_budget(1);
    assert_eq!(first.reclaimed(), 1);
    assert!(first.may_have_remaining());
    assert!(!cache.contains(optimizing));
    assert!(cache.contains(baseline));

    let second = cache.poll_reclamation_with_budget(1);
    assert_eq!(second.reclaimed(), 1);
    assert!(!second.may_have_remaining());
    assert!(!cache.contains(baseline));
}

#[test]
fn dependency_release_is_reconsidered_during_transactional_multi_eviction() {
    let mut cache = CodeCache::new(2);
    let baseline = artifact_key(1, 1, Tier::Baseline);
    let optimizing = baseline.with_tier(Tier::Optimizing);
    let unrelated = artifact_key(2, 1, Tier::Baseline);
    cache.insert(artifact_with_code(baseline, 1)).unwrap();
    cache.insert(artifact_with_code(optimizing, 1)).unwrap();

    let insertion = cache.insert(artifact_with_code(unrelated, 2)).unwrap();

    assert_eq!(insertion.evictions(), &[optimizing, baseline]);
    assert!(!cache.contains(baseline));
    assert!(!cache.contains(optimizing));
    assert!(cache.contains(unrelated));
    assert_eq!(cache.charged_bytes(), 2);
}

#[test]
fn failed_dependency_aware_eviction_plan_is_transactional() {
    let mut cache = CodeCache::new(2);
    let baseline = artifact_key(1, 1, Tier::Baseline);
    let optimizing = baseline.with_tier(Tier::Optimizing);
    let unrelated = artifact_key(2, 1, Tier::Baseline);
    cache.insert(artifact_with_code(baseline, 1)).unwrap();
    cache.insert(artifact_with_code(optimizing, 1)).unwrap();
    let baseline_pin = cache.pin(baseline).unwrap();

    assert_eq!(
        cache.insert(artifact_with_code(unrelated, 2)).unwrap_err(),
        rquickjs_jit::code_cache::CacheError::AllArtifactsPinned
    );
    assert!(cache.contains(baseline));
    assert!(cache.contains(optimizing));
    assert!(!cache.contains(unrelated));
    assert_eq!(cache.charged_bytes(), 2);
    assert_eq!(baseline_pin.key(), baseline);
}

fn successful_completion(request: &rquickjs_jit::runtime::CompileRequest) -> CompileCompletion {
    CompileCompletion {
        key: request.key(),
        requested_tier: request.tier(),
        artifact_key: request.artifact_key(),
        attempt_id: request.attempt_id(),
        result: Ok(artifact_with_code(request.artifact_key(), 1)),
    }
}

#[test]
fn request_queue_saturation_is_bounded_and_observable() {
    let mut coordinator = Coordinator::with_limits(1, 1, 4, 3);
    let first = FunctionKey::new(1, 1);
    let rejected = FunctionKey::new(2, 1);
    coordinator
        .queue(first, Tier::Baseline, coordinator_snapshot())
        .unwrap();

    assert_eq!(
        coordinator.queue(rejected, Tier::Baseline, coordinator_snapshot()),
        Err(QueueError::Full)
    );
    assert_eq!(
        coordinator.state(first),
        CompileState::Queued(Tier::Baseline)
    );
    assert_eq!(coordinator.state(rejected), CompileState::Cold);
    assert_eq!(coordinator.metrics().queued, 1);
    assert_eq!(coordinator.metrics().queue_saturated, 1);
}

#[test]
fn completion_channel_is_bounded_and_drained_only_by_coordinator() {
    let mut coordinator = Coordinator::with_limits(2, 1, 4, 3);
    let sender = coordinator.completion_sender();
    for key in [FunctionKey::new(1, 1), FunctionKey::new(2, 1)] {
        coordinator
            .queue(key, Tier::Baseline, coordinator_snapshot())
            .unwrap();
    }
    let first = coordinator.begin_next().unwrap();
    let second = coordinator.begin_next().unwrap();
    sender.try_send(successful_completion(&first)).unwrap();
    assert!(matches!(
        sender.try_send(successful_completion(&second)),
        Err(CompletionSendError::Full(_))
    ));
    assert_eq!(coordinator.metrics().queue_saturated, 0);
    assert_eq!(coordinator.metrics().completion_queue_saturated, 1);

    assert_eq!(
        coordinator.state(first.key()),
        CompileState::Compiling(Tier::Baseline)
    );
    let poll = coordinator.drain_completions();
    assert_eq!(poll.drained(), 1);
    assert!(!poll.may_have_remaining());
    assert_eq!(
        coordinator.state(first.key()),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(
        coordinator.state(second.key()),
        CompileState::Compiling(Tier::Baseline)
    );
}

#[test]
fn completion_drain_obeys_exact_budget_and_reports_remaining_work() {
    let mut coordinator = Coordinator::with_limits(3, 3, 4, 16);
    let sender = coordinator.completion_sender();
    let mut requests = Vec::new();
    for id in 1..=3 {
        let key = FunctionKey::new(id, 1);
        coordinator
            .queue(key, Tier::Baseline, coordinator_snapshot())
            .unwrap();
        requests.push(coordinator.begin_next().unwrap());
    }
    for request in &requests {
        sender.try_send(successful_completion(request)).unwrap();
    }

    let first_poll = coordinator.drain_completions_with_budget(2);
    assert_eq!(first_poll.drained(), 2);
    assert!(first_poll.may_have_remaining());
    assert_eq!(
        coordinator.state(requests[2].key()),
        CompileState::Compiling(Tier::Baseline)
    );

    let second_poll = coordinator.drain_completions_with_budget(2);
    assert_eq!(second_poll.drained(), 1);
    assert!(!second_poll.may_have_remaining());
    assert_eq!(
        coordinator.state(requests[2].key()),
        CompileState::Installed(Tier::Baseline)
    );
}

#[test]
fn runtime_poll_shares_budget_between_completions_and_reclamation() {
    let mut coordinator = Coordinator::with_limits(4, 4, 4, 4);
    for id in 1..=3 {
        let key = FunctionKey::new(id, 1);
        coordinator
            .queue(key, Tier::Baseline, coordinator_snapshot())
            .unwrap();
        let request = coordinator.begin_next().unwrap();
        coordinator.complete(successful_completion(&request));
        coordinator.retire(key);
    }
    assert_eq!(coordinator.cache_len(), 3);

    let live = FunctionKey::new(4, 1);
    coordinator
        .queue(live, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    coordinator
        .completion_sender()
        .try_send(successful_completion(&request))
        .unwrap();

    let first_poll = coordinator.drain_completions_with_budget(2);
    assert_eq!(first_poll.drained(), 0);
    assert_eq!(first_poll.reclaimed(), 2);
    assert!(first_poll.may_have_remaining());
    assert_eq!(coordinator.cache_len(), 1);

    let second_poll = coordinator.drain_completions_with_budget(2);
    assert_eq!(second_poll.drained(), 1);
    assert_eq!(second_poll.reclaimed(), 1);
    assert!(!second_poll.may_have_remaining());
    assert_eq!(coordinator.cache_len(), 1);
    assert!(coordinator.pin(live, Tier::Baseline).is_some());
}

#[test]
fn shutdown_closes_publication_and_retires_pending_work() {
    let mut coordinator = Coordinator::with_limits(1, 1, 4, 3);
    let sender = coordinator.completion_sender();
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();

    coordinator.shutdown();
    assert_eq!(coordinator.state(key), CompileState::Retired);
    assert!(matches!(
        sender.try_send(successful_completion(&request)),
        Err(CompletionSendError::Closed(_))
    ));
    assert_eq!(
        coordinator.queue(
            FunctionKey::new(2, 1),
            Tier::Baseline,
            coordinator_snapshot()
        ),
        Err(QueueError::Shutdown)
    );
    assert_eq!(coordinator.metrics().installed, 0);
}

#[test]
fn mismatched_artifact_identity_leaves_legitimate_attempt_in_flight() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    let wrong_key = ArtifactKey {
        config_fingerprint: request.artifact_key().config_fingerprint + 1,
        ..request.artifact_key()
    };

    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: request.artifact_key(),
        attempt_id: request.attempt_id(),
        result: Ok(CompiledArtifact::empty(wrong_key)),
    });

    assert_eq!(
        coordinator.state(key),
        CompileState::Compiling(Tier::Baseline)
    );
    coordinator.complete(successful_completion(&request));
    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().compile_failures, 0);
    assert_eq!(coordinator.metrics().installed, 1);
}

#[test]
fn mismatched_failure_identity_leaves_legitimate_attempt_in_flight() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    let wrong_key = ArtifactKey {
        target_isa: request.artifact_key().target_isa + 1,
        ..request.artifact_key()
    };

    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        attempt_id: request.attempt_id(),
        artifact_key: wrong_key,
        result: Err(CompileFailure::UnsupportedOpcode),
    });

    assert_eq!(
        coordinator.state(key),
        CompileState::Compiling(Tier::Baseline)
    );
    coordinator.complete(successful_completion(&request));
    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().compile_failures, 0);
}

#[test]
fn old_same_key_retry_completion_cannot_satisfy_new_attempt() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let first = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: first.artifact_key(),
        attempt_id: first.attempt_id(),
        result: Err(CompileFailure::UnsupportedOpcode),
    });
    let CompileState::Backoff { retry_after, .. } = coordinator.state(key) else {
        unreachable!()
    };
    coordinator.advance_clock(retry_after);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let second = coordinator.begin_next().unwrap();

    coordinator.complete(successful_completion(&first));
    assert_eq!(
        coordinator.state(key),
        CompileState::Compiling(Tier::Baseline)
    );
    coordinator.complete(successful_completion(&second));

    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(coordinator.metrics().stale_results, 1);
}

#[test]
fn newer_generation_retires_older_in_flight_compilation() {
    let mut coordinator = coordinator(4);
    let old = FunctionKey::new(1, 1);
    let new = FunctionKey::new(1, 2);
    coordinator
        .queue(old, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let old_request = coordinator.begin_next().unwrap();
    coordinator
        .queue(new, Tier::Baseline, coordinator_snapshot())
        .unwrap();

    coordinator.complete(successful_completion(&old_request));

    assert_eq!(coordinator.state(old), CompileState::Retired);
    assert_eq!(coordinator.state(new), CompileState::Queued(Tier::Baseline));
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().installed, 0);
}

#[test]
fn advancing_retirement_watermark_unpublishes_every_lower_generation() {
    let mut coordinator = coordinator(4);
    let installed = FunctionKey::new(1, 1);
    coordinator
        .queue(installed, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&request));
    assert!(coordinator.pin(installed, Tier::Baseline).is_some());

    coordinator.retire(FunctionKey::new(installed.id, 2));

    assert_eq!(coordinator.state(installed), CompileState::Retired);
    assert!(coordinator.pin(installed, Tier::Baseline).is_none());
}

#[test]
fn full_new_generation_admission_still_retires_older_in_flight_work() {
    let mut coordinator = Coordinator::with_limits(1, 1, 4, 4);
    let old = FunctionKey::new(1, 1);
    let new = FunctionKey::new(1, 2);
    let blocker = FunctionKey::new(2, 1);
    coordinator
        .queue(old, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let old_request = coordinator.begin_next().unwrap();
    coordinator
        .queue(blocker, Tier::Baseline, coordinator_snapshot())
        .unwrap();

    assert_eq!(
        coordinator.queue(new, Tier::Baseline, coordinator_snapshot()),
        Err(QueueError::Full)
    );
    assert_eq!(coordinator.state(old), CompileState::Retired);

    coordinator.complete(successful_completion(&old_request));
    assert!(coordinator.pin(old, Tier::Baseline).is_none());
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().installed, 0);
}

#[test]
fn coordinator_retries_cache_install_while_all_artifacts_are_pinned() {
    let mut coordinator = Coordinator::with_limits(2, 2, 4, 1);
    let first = FunctionKey::new(1, 1);
    coordinator
        .queue(first, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let first_request = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&first_request));
    let first_pin = coordinator
        .pin(first, Tier::Baseline)
        .expect("installed baseline is pinnable");

    let second = FunctionKey::new(2, 1);
    coordinator
        .queue(second, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let second_request = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&second_request));
    assert!(matches!(
        coordinator.state(second),
        CompileState::Backoff { attempts: 1, .. }
    ));
    assert_eq!(coordinator.metrics().installed, 1);
    assert_eq!(coordinator.cache_len(), 1);

    drop(first_pin);
    let CompileState::Backoff { retry_after, .. } = coordinator.state(second) else {
        unreachable!()
    };
    coordinator.advance_clock(retry_after);
    coordinator
        .queue(second, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let retry = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&retry));

    assert_eq!(coordinator.state(first), CompileState::Cold);
    assert_eq!(
        coordinator.state(second),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(coordinator.metrics().installed, 2);
    assert_eq!(coordinator.metrics().evicted, 1);
}

#[test]
fn optimizing_install_replaces_entry_but_retains_baseline_target() {
    let mut coordinator = Coordinator::with_limits(2, 2, 4, 3);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&baseline));

    coordinator
        .queue(key, Tier::Optimizing, coordinator_snapshot())
        .unwrap();
    let optimizing = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&optimizing));

    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Optimizing)
    );
    let baseline_pin = coordinator
        .pin(key, Tier::Baseline)
        .expect("Tier 1 remains a deopt target");
    let optimizing_pin = coordinator
        .pin(key, Tier::Optimizing)
        .expect("Tier 2 is the current entry");
    coordinator.retire(key);
    assert!(coordinator.pin(key, Tier::Baseline).is_none());
    assert!(coordinator.pin(key, Tier::Optimizing).is_none());
    assert_eq!(baseline_pin.key().tier, Tier::Baseline);
    assert_eq!(optimizing_pin.key().tier, Tier::Optimizing);
    drop(baseline_pin);
    drop(optimizing_pin);
    assert_eq!(coordinator.poll_cache_reclamation(), 2);
    assert_eq!(coordinator.cache_len(), 0);
}

#[test]
fn optimizing_queue_requires_an_installed_baseline() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);

    assert_eq!(
        coordinator.queue(key, Tier::Optimizing, coordinator_snapshot()),
        Err(QueueError::NotReady)
    );
    assert_eq!(coordinator.state(key), CompileState::Cold);
}

#[test]
fn baseline_retry_cannot_switch_to_optimizing_tier() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Baseline,
        artifact_key: request.artifact_key(),
        attempt_id: request.attempt_id(),
        result: Err(CompileFailure::UnsupportedOpcode),
    });
    let CompileState::Backoff { retry_after, .. } = coordinator.state(key) else {
        unreachable!()
    };
    coordinator.advance_clock(retry_after);

    assert_eq!(
        coordinator.queue(key, Tier::Optimizing, coordinator_snapshot()),
        Err(QueueError::NotReady)
    );
    assert!(matches!(
        coordinator.state(key),
        CompileState::Backoff { attempts: 1, .. }
    ));
}

#[test]
fn optimizing_failure_preserves_baseline_and_its_own_retry_state() {
    let mut coordinator = Coordinator::with_limits(2, 2, 4, 16);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&baseline));
    coordinator
        .queue(key, Tier::Optimizing, coordinator_snapshot())
        .unwrap();
    let optimizing = coordinator.begin_next().unwrap();

    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Optimizing,
        artifact_key: optimizing.artifact_key(),
        attempt_id: optimizing.attempt_id(),
        result: Err(CompileFailure::UnsupportedOpcode),
    });

    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
    assert!(matches!(
        coordinator.tier_state(key, Tier::Optimizing),
        CompileState::Backoff { attempts: 1, .. }
    ));
    assert!(coordinator.pin(key, Tier::Baseline).is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn deterministic_harness_rejects_stale_and_blacklists_per_generation() {
    use rquickjs_jit::test_support::Harness;

    let stale = Harness::new();
    let stale_key = FunctionKey::new(7, 3);
    stale.queue(stale_key);
    stale.retire(stale_key);
    stale.complete(stale_key, CompiledArtifact::fake(Tier::Baseline));
    assert_eq!(stale.state(stale_key), CompileState::Retired);
    assert_eq!(stale.metrics().stale_results, 1);
    assert_eq!(stale.metrics().installed, 0);

    let failing = Harness::with_max_attempts(4);
    let failed_key = FunctionKey::new(9, 1);
    for _ in 0..4 {
        failing.fail(failed_key, CompileFailure::UnsupportedOpcode);
    }
    assert_eq!(failing.state(failed_key), CompileState::Blacklisted);
    assert_eq!(failing.state(FunctionKey::new(9, 2)), CompileState::Cold);
}

#[test]
fn compiled_artifact_owns_code_and_all_runtime_metadata() {
    let key = artifact_key(1, 1, Tier::Baseline);
    let artifact = CompiledArtifact::from_parts(
        key,
        CodeAllocation::inert(vec![0xaa, 0xbb]),
        vec![Relocation::new(1, 22, -3)],
        vec![StackMap::new(4, vec![0, 2])],
        vec![FrameState::new(4, 9, vec![1, 3])],
        vec![ArtifactDependency::new(FunctionKey::new(8, 5))],
    );

    assert_eq!(artifact.key(), key);
    assert_eq!(artifact.code().bytes(), &[0xaa, 0xbb]);
    assert!(!artifact.code().is_executable());
    assert_eq!(artifact.relocations(), &[Relocation::new(1, 22, -3)]);
    assert_eq!(artifact.stack_maps(), &[StackMap::new(4, vec![0, 2])]);
    assert_eq!(
        artifact.frame_states(),
        &[FrameState::new(4, 9, vec![1, 3])]
    );
    assert_eq!(
        artifact.dependencies(),
        &[ArtifactDependency::new(FunctionKey::new(8, 5))]
    );
    assert_eq!(artifact.benefit().executions, 0);
    assert_eq!(artifact.benefit().score, 0);
}

#[test]
fn coordinator_propagates_full_environment_into_artifact_identity() {
    let environment = ArtifactEnvironment {
        runtime_id: 10,
        target_isa: 20,
        cpu_features: 30,
        abi_fingerprint: 40,
        config_fingerprint: 50,
    };
    let mut coordinator = Coordinator::with_environment(1, 1, 4, 3, environment);
    let key = FunctionKey::new(60, 70);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();

    let artifact = coordinator.begin_next().unwrap().artifact_key();
    assert_eq!(artifact.runtime_id, 10);
    assert_eq!(artifact.function_id, 60);
    assert_eq!(artifact.generation, 70);
    assert_eq!(artifact.target_isa, 20);
    assert_eq!(artifact.cpu_features, 30);
    assert_eq!(artifact.abi_fingerprint, 40);
    assert_eq!(artifact.config_fingerprint, 50);
    assert_eq!(artifact.source_revision, rquickjs_jit::abi::SOURCE_REVISION);
    assert_eq!(
        artifact.opcode_fingerprint,
        rquickjs_jit::abi::OPCODE_FINGERPRINT
    );
}

#[test]
fn coordinator_rejects_snapshot_from_a_different_function_generation() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let verified = context.with(|ctx| {
        let function: Function<'_> = ctx.eval("(function captured() { return 1 })").unwrap();
        unsafe { CompileSnapshot::capture_raw(ctx.as_raw().as_ptr(), function.as_value().as_raw()) }
            .unwrap()
            .verify(VerifyLimits::default())
            .unwrap()
    });
    let captured = FunctionKey::new(
        verified.snapshot().function_id(),
        verified.snapshot().generation(),
    );
    let wrong = FunctionKey::new(captured.id.saturating_add(1), captured.generation);
    let mut coordinator = coordinator(4);

    assert_eq!(
        coordinator.queue(wrong, Tier::Baseline, verified),
        Err(QueueError::SnapshotIdentity)
    );
    assert_eq!(coordinator.state(wrong), CompileState::Cold);
}

#[test]
fn wrong_tier_completion_does_not_consume_legitimate_in_flight_request() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();

    coordinator.complete(CompileCompletion {
        key,
        requested_tier: Tier::Optimizing,
        artifact_key: request.artifact_key(),
        attempt_id: request.attempt_id(),
        result: Err(CompileFailure::Cancelled),
    });
    assert_eq!(
        coordinator.state(key),
        CompileState::Compiling(Tier::Baseline)
    );
    coordinator.complete(successful_completion(&request));

    assert_eq!(
        coordinator.state(key),
        CompileState::Installed(Tier::Baseline)
    );
    assert_eq!(coordinator.metrics().stale_results, 1);
    assert_eq!(coordinator.metrics().compile_failures, 0);
}

#[test]
fn evicting_optimizing_artifact_restores_baseline_install_state() {
    let mut coordinator = Coordinator::with_limits(3, 3, 4, 2);
    let tiered = FunctionKey::new(1, 1);
    coordinator
        .queue(tiered, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let baseline = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&baseline));
    coordinator
        .queue(tiered, Tier::Optimizing, coordinator_snapshot())
        .unwrap();
    let optimizing = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&optimizing));

    let newcomer = FunctionKey::new(2, 1);
    coordinator
        .queue(newcomer, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let newcomer_request = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&newcomer_request));

    assert_eq!(
        coordinator.state(tiered),
        CompileState::Installed(Tier::Baseline)
    );
    assert!(coordinator.pin(tiered, Tier::Baseline).is_some());
    assert!(coordinator.pin(tiered, Tier::Optimizing).is_none());
    assert_eq!(coordinator.metrics().evicted, 1);
}

#[test]
fn released_invalidated_artifact_is_reclaimed_before_live_eviction() {
    let mut cache = CodeCache::new(2);
    let retired = artifact_key(1, 1, Tier::Baseline);
    let live = artifact_key(2, 1, Tier::Baseline);
    let newcomer = artifact_key(3, 1, Tier::Baseline);
    cache.insert(artifact_with_code(retired, 1)).unwrap();
    cache.insert(artifact_with_code(live, 1)).unwrap();
    cache.record_benefit(retired, 100).unwrap();
    let pin = cache.pin(retired).unwrap();
    assert_eq!(cache.invalidate(FunctionKey::new(1, 1)), 0);

    drop(pin);
    cache.insert(artifact_with_code(newcomer, 1)).unwrap();
    assert!(!cache.contains(retired));
    assert!(cache.contains(live));
    assert!(cache.contains(newcomer));
}

#[test]
fn final_pin_drop_marks_invalidated_artifact_for_explicit_poll_reclamation() {
    let mut cache = CodeCache::new(1);
    let retired = artifact_key(1, 1, Tier::Baseline);
    cache.insert(artifact_with_code(retired, 1)).unwrap();
    let pin = cache.pin(retired).unwrap();
    assert_eq!(cache.invalidate(FunctionKey::new(1, 1)), 0);

    drop(pin);

    assert!(cache.contains(retired));
    assert_eq!(cache.poll_reclamation(), 1);
    assert!(!cache.contains(retired));
    assert_eq!(cache.charged_bytes(), 0);
}

#[test]
fn shutdown_unpublishes_and_drains_installed_artifacts() {
    let mut coordinator = coordinator(4);
    let key = FunctionKey::new(1, 1);
    coordinator
        .queue(key, Tier::Baseline, coordinator_snapshot())
        .unwrap();
    let request = coordinator.begin_next().unwrap();
    coordinator.complete(successful_completion(&request));
    assert!(coordinator.pin(key, Tier::Baseline).is_some());

    coordinator.shutdown();

    assert_eq!(coordinator.state(key), CompileState::Retired);
    assert!(coordinator.pin(key, Tier::Baseline).is_none());
    assert_eq!(coordinator.cache_len(), 0);
}

unsafe extern "C" fn native_done(frame: *mut qjs::JSJitExecFrame) -> qjs::JSJitExit {
    unsafe {
        (*frame).result = qjs::JS_MKVAL(qjs::JS_TAG_INT, 42);
    }
    qjs::JSJitExit::done()
}

struct AlwaysNativeBackend;

unsafe impl JitBackend for AlwaysNativeBackend {
    fn acquire_entry(&mut self, _id: u64, _generation: u64, pc: u32) -> qjs::JSJitEntryHandle {
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: (pc == 0).then_some(native_done),
            pin: Box::into_raw(Box::new(0_u8)).cast(),
            stack_map_count: 0,
            helper_abi_version: qjs::QJSJIT_HELPER_ABI_VERSION,
        }
    }

    fn release_entry(&mut self, entry: qjs::JSJitEntryHandle) {
        unsafe { drop(Box::from_raw(entry.pin.cast::<u8>())) };
    }
}

struct ToggleNativeBackend {
    enabled: Arc<AtomicBool>,
}

unsafe impl JitBackend for ToggleNativeBackend {
    fn acquire_entry(&mut self, _id: u64, _generation: u64, pc: u32) -> qjs::JSJitEntryHandle {
        let active = self.enabled.load(Ordering::Acquire) && pc == 0;
        qjs::JSJitEntryHandle {
            struct_size: std::mem::size_of::<qjs::JSJitEntryHandle>() as u32,
            reserved: 0,
            entry: active.then_some(native_done),
            pin: if active {
                Box::into_raw(Box::new(0_u8)).cast()
            } else {
                std::ptr::null_mut()
            },
            stack_map_count: 0,
            helper_abi_version: if active {
                qjs::QJSJIT_HELPER_ABI_VERSION
            } else {
                0
            },
        }
    }

    fn release_entry(&mut self, entry: qjs::JSJitEntryHandle) {
        unsafe { drop(Box::from_raw(entry.pin.cast::<u8>())) };
    }
}

#[test]
fn backend_attached_before_the_first_context_executes_after_context_initialization() {
    let runtime = Runtime::new().unwrap();
    let enabled = Arc::new(AtomicBool::new(false));
    let guard = runtime
        .attach_jit_backend(ToggleNativeBackend {
            enabled: Arc::clone(&enabled),
        })
        .expect("attach before any context exists");
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
    });
    enabled.store(true, Ordering::Release);

    let native = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });

    assert_eq!(native, 42);
    drop(guard);
}

#[test]
fn guard_drop_leaves_runtime_clones_and_contexts_interpreter_only() {
    let runtime = Runtime::new().unwrap();
    let runtime_clone = runtime.clone();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
    });
    let guard = runtime
        .attach_jit_backend(AlwaysNativeBackend)
        .expect("attach native fixture");

    let native = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });
    assert_eq!(native, 42);

    drop(guard);
    drop(runtime);
    runtime_clone.run_gc();
    let interpreted = context.with(|ctx| {
        let function: Function<'_> = ctx.globals().get("target").unwrap();
        function.call::<_, i32>(()).unwrap()
    });
    assert_eq!(interpreted, 1);
}

struct PendingWork {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for PendingWork {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("queued_work_drop");
    }
}

struct OrderedBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    pending: Option<PendingWork>,
}

unsafe impl JitBackend for OrderedBackend {
    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push("detach");
        drop(self.pending.take());
    }
}

impl Drop for OrderedBackend {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("backend_drop");
    }
}

#[test]
fn raw_runtime_forces_detach_and_drains_work_while_guard_survives() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let guard = runtime
        .attach_jit_backend(OrderedBackend {
            events: Arc::clone(&events),
            pending: Some(PendingWork {
                events: Arc::clone(&events),
            }),
        })
        .unwrap();

    drop(runtime);
    assert!(events.lock().unwrap().is_empty());
    drop(context);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["detach", "queued_work_drop", "backend_drop"]
    );

    drop(guard);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["detach", "queued_work_drop", "backend_drop"]
    );
}

struct RegistryValueDrop {
    events: Arc<Mutex<Vec<&'static str>>>,
    registry: JitFunctionRegistry,
}

impl Drop for RegistryValueDrop {
    fn drop(&mut self) {
        let event = if self.registry.is_attached() {
            "registry_value_drop_while_attached"
        } else {
            "registry_value_drop_after_detach"
        };
        self.events.lock().unwrap().push(event);
    }
}

struct RegistryBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    pending: Option<PendingWork>,
    registry: Arc<Mutex<Option<JitFunctionRegistry>>>,
}

unsafe impl JitBackend for RegistryBackend {
    fn runtime_attached(&mut self, registry: JitFunctionRegistry) {
        *self.registry.lock().unwrap() = Some(registry);
    }

    fn runtime_detach(&mut self) {
        self.events.lock().unwrap().push("detach");
        drop(self.pending.take());
    }
}

impl Drop for RegistryBackend {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("backend_drop");
    }
}

#[test]
fn runtime_owned_registry_releases_functions_before_free_while_guard_survives() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let registry_slot = Arc::new(Mutex::new(None));
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let guard = runtime
        .attach_jit_backend(RegistryBackend {
            events: Arc::clone(&events),
            pending: Some(PendingWork {
                events: Arc::clone(&events),
            }),
            registry: Arc::clone(&registry_slot),
        })
        .unwrap();
    let registry = registry_slot
        .lock()
        .unwrap()
        .clone()
        .expect("runtime_attached supplied a registry handle");

    context.with(|ctx| {
        let value_drop = RegistryValueDrop {
            events: Arc::clone(&events),
            registry: registry.clone(),
        };
        let host = Function::new(ctx.clone(), move || {
            let _ = &value_drop;
        })
        .unwrap();
        ctx.globals().set("__jitRegistryValue", host).unwrap();
        ctx.eval::<(), _>(
            r#"
            globalThis.target = (function make(value) {
                return function target() { return value };
            })(globalThis.__jitRegistryValue);
            delete globalThis.__jitRegistryValue;
            "#,
        )
        .unwrap();

        let function: Function<'_> = ctx.globals().get("target").unwrap();
        let snapshot = unsafe {
            CompileSnapshot::capture_raw(ctx.as_raw().as_ptr(), function.as_value().as_raw())
        }
        .unwrap();
        registry
            .retain_function(
                &ctx,
                &function,
                snapshot.function_id(),
                snapshot.generation(),
            )
            .unwrap();
        assert_eq!(registry.retained_len(&ctx).unwrap(), 1);
        ctx.eval::<(), _>("delete globalThis.target").unwrap();
    });
    assert!(events.lock().unwrap().is_empty());

    drop(runtime);
    assert!(events.lock().unwrap().is_empty());
    drop(context);
    events.lock().unwrap().push("runtime_drop_returned");

    assert!(!registry.is_attached());
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "detach",
            "queued_work_drop",
            "registry_value_drop_after_detach",
            "backend_drop",
            "runtime_drop_returned",
        ]
    );
    drop(guard);
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&"runtime_drop_returned")
    );
}

struct RegistryOnlyBackend {
    registry: Arc<Mutex<Option<JitFunctionRegistry>>>,
}

unsafe impl JitBackend for RegistryOnlyBackend {
    fn runtime_attached(&mut self, registry: JitFunctionRegistry) {
        *self.registry.lock().unwrap() = Some(registry);
    }
}

#[test]
fn registry_mutation_and_reads_are_synchronized_across_safe_scoped_threads() {
    const RETAINS: u64 = 16_384;

    let registry_slot = Arc::new(Mutex::new(None));
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("globalThis.target = function target() { return 1 }")
            .unwrap();
    });
    let guard = runtime
        .attach_jit_backend(RegistryOnlyBackend {
            registry: Arc::clone(&registry_slot),
        })
        .unwrap();
    let registry = registry_slot
        .lock()
        .unwrap()
        .clone()
        .expect("runtime_attached supplied a registry handle");

    context.with(|ctx| {
        let writer_ctx = ctx.clone();
        let reader_ctx = ctx.clone();
        let writer_registry = registry.clone();
        let reader_registry = registry.clone();
        let start = Arc::new(Barrier::new(2));
        let settled = Arc::new(Barrier::new(2));
        let finished = Arc::new(AtomicBool::new(false));

        thread::scope(|scope| {
            let writer_start = Arc::clone(&start);
            let writer_settled = Arc::clone(&settled);
            let writer_finished = Arc::clone(&finished);
            let writer = scope.spawn(move || {
                let function: Function<'_> = writer_ctx.globals().get("target").unwrap();
                writer_start.wait();
                for id in 1..=RETAINS {
                    writer_registry
                        .retain_function(&writer_ctx, &function, id, 1)
                        .unwrap();
                    if id % 64 == 0 {
                        thread::yield_now();
                    }
                }
                writer_finished.store(true, Ordering::Release);
                writer_settled.wait();
                drop(function);
                writer_ctx
            });

            let reader_start = Arc::clone(&start);
            let reader_settled = Arc::clone(&settled);
            let reader = scope.spawn(move || {
                reader_start.wait();
                while !finished.load(Ordering::Acquire) {
                    let len = reader_registry.retained_len(&reader_ctx).unwrap();
                    assert!(len <= RETAINS as usize);
                    thread::yield_now();
                }
                assert_eq!(
                    reader_registry.retained_len(&reader_ctx).unwrap(),
                    RETAINS as usize
                );
                reader_settled.wait();
                reader_ctx
            });

            let writer_ctx = writer.join().unwrap();
            let reader_ctx = reader.join().unwrap();
            drop(writer_ctx);
            drop(reader_ctx);
        });

        assert_eq!(registry.retained_len(&ctx).unwrap(), RETAINS as usize);
    });

    drop(guard);
    assert!(!registry.is_attached());
}
