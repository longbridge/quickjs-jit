use rquickjs_core::{Context, Runtime};
use rquickjs_jit::correctness::{
    canonical_observation_source, classify_features, classify_features_against,
    classify_features_with_config, compose_test262_program, discover_test262, parse_test262,
    Exclusion, ExclusionManifest, FeatureDisposition, NegativePhase, RunMode, StructuredProgram,
    SuiteReport, Test262Variant,
};
use std::fs;

const FULL_METADATA: &str = r#"
/*---
description: metadata is not optional
flags: [onlyStrict, async, module]
includes: [assert.js, sta.js]
features: [BigInt, Symbol]
negative:
  phase: runtime
  type: TypeError
---*/
throw new TypeError();
"#;

#[test]
fn parses_real_test262_yaml_and_derives_an_exact_variant() {
    let test = parse_test262("language/example.js", FULL_METADATA).unwrap();
    assert_eq!(test.includes(), ["assert.js", "sta.js"]);
    assert_eq!(test.features(), ["BigInt", "Symbol"]);
    assert_eq!(test.negative().unwrap().phase(), NegativePhase::Runtime);
    assert_eq!(test.negative().unwrap().error_type(), "TypeError");
    assert_eq!(test.variants(), [Test262Variant::StrictModuleAsync]);
    assert_eq!(test.body(), "throw new TypeError();\n");
    assert_eq!(test.source(), FULL_METADATA);
    assert_eq!(
        &test.source()[test.body_range()],
        "throw new TypeError();\n"
    );
    assert!(test.source()[test.metadata_range()].starts_with("/*---"));
}

#[test]
fn parser_preserves_raw_lexemes_and_only_removes_the_frontmatter_from_body() {
    let source =
        "// license\r\n/*---\r\nflags: [raw]\r\n---*/\r\n'\\u0061'; // keep CRLF and escape\r\n";
    let test = parse_test262("raw.js", source).unwrap();
    assert_eq!(test.source().as_bytes(), source.as_bytes());
    assert_eq!(test.body(), "'\\u0061'; // keep CRLF and escape\r\n");
    assert_eq!(&test.source()[test.body_range()], test.body());
}

#[test]
fn report_refuses_zero_executed_variants_and_modes_are_strict() {
    assert_eq!(
        "force-tier2".parse::<RunMode>().unwrap(),
        RunMode::ForceTier2
    );
    assert!("tier1-ish".parse::<RunMode>().is_err());
    let report = SuiteReport::new("test262", "deadbeef", RunMode::Automatic, vec![]);
    assert!(report.validate().unwrap_err().contains("zero cases"));
}

#[test]
fn forced_reports_allow_explicitly_ineligible_skips_but_never_native_less_passes() {
    use rquickjs_jit::correctness::{CaseReport, CaseStatus, NativeEvidence};
    let skip = CaseReport {
        path: "unsupported.js".into(),
        variant: Test262Variant::RawScript,
        status: CaseStatus::Skip,
        duration_ms: 0,
        negative_phase: None,
        negative_type: None,
        skip_reason: Some("tier1-ineligible: eval opcode".into()),
        error: None,
        native: NativeEvidence {
            native_entries: 0,
            tier2_entries: 0,
            native_exits: 0,
            unexpected_fallbacks: 0,
            opcode_ids: vec![],
            helper_ids: vec![],
        },
    };
    SuiteReport::new("test262", "deadbeef", RunMode::ForceTier1, vec![skip])
        .validate()
        .unwrap();
}

#[test]
fn shard_merge_rejects_missing_duplicate_and_zero_case_shards() {
    use rquickjs_jit::correctness::{CaseReport, CaseStatus, NativeEvidence};
    let case = |path: &str| CaseReport {
        path: path.into(),
        variant: Test262Variant::RawScript,
        status: CaseStatus::Pass,
        duration_ms: 0,
        negative_phase: None,
        negative_type: None,
        skip_reason: None,
        error: None,
        native: NativeEvidence {
            native_entries: 0,
            tier2_entries: 0,
            native_exits: 0,
            unexpected_fallbacks: 0,
            opcode_ids: vec![],
            helper_ids: vec![],
        },
    };
    let shard = |index, cases| {
        SuiteReport::new("test262", "rev", RunMode::Automatic, cases).with_discovery(2, index, 2)
    };
    assert!(SuiteReport::merge_shards(vec![shard(0, vec![case("a")])])
        .unwrap_err()
        .contains("missing shard"));
    assert!(
        SuiteReport::merge_shards(vec![shard(0, vec![case("a")]), shard(0, vec![case("b")])])
            .unwrap_err()
            .contains("duplicate shard")
    );
    assert!(
        SuiteReport::merge_shards(vec![shard(0, vec![]), shard(1, vec![case("b")])])
            .unwrap_err()
            .contains("zero cases")
    );
    let merged =
        SuiteReport::merge_shards(vec![shard(1, vec![case("b")]), shard(0, vec![case("a")])])
            .unwrap();
    assert_eq!(
        merged
            .cases
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn structured_programs_are_seeded_terminating_and_shrinkable() {
    let program = StructuredProgram::generate(0x1234_5678, 24);
    assert_eq!(program, StructuredProgram::generate(0x1234_5678, 24));
    assert!(program.source().contains("for(let i=0;i<"));
    assert!(program.source().contains("Proxy"));
    assert!(program.source().contains("events.join(',')"));
    assert!(!program.source().contains("return {total"));
    let shrunk = program.shrink();
    assert!(shrunk.fuel() < program.fuel());
    assert_eq!(shrunk.seed(), program.seed());
}

#[test]
fn every_fuzz_seed_has_current_version_and_opcode_fingerprint() {
    const PREFIX: &[u8] = b"QJSJFZ01:05d5c0867521c077:";
    for seed in [
        include_bytes!("../fuzz/corpus/snapshot/v1-opcodes").as_slice(),
        include_bytes!("../fuzz/corpus/verifier/v1-opcodes"),
        include_bytes!("../fuzz/corpus/differential/v1-opcodes"),
        include_bytes!("../fuzz/corpus/frame_state/v1-opcodes"),
        include_bytes!("../fuzz/corpus/lowering/v1-opcodes"),
        include_bytes!("../fuzz/corpus/relocations/v1-opcodes"),
    ] {
        assert!(seed.starts_with(PREFIX));
        assert!(seed.len() > PREFIX.len());
    }
}

#[test]
fn ordinary_scripts_run_both_strict_variants_but_raw_runs_once() {
    let ordinary = parse_test262("ordinary.js", "/*---\n---*/\n1 + 1;\n").unwrap();
    assert_eq!(
        ordinary.variants(),
        [Test262Variant::SloppyScript, Test262Variant::StrictScript]
    );
    let raw = parse_test262("raw.js", "/*---\nflags: [raw]\n---*/\n1 + 1;\n").unwrap();
    assert_eq!(raw.variants(), [Test262Variant::RawScript]);
}

#[test]
fn raw_programs_receive_exact_body_without_harness_or_strict_directive() {
    let raw = parse_test262(
        "raw.js",
        "/*---\nflags: [raw]\nincludes: [assert.js]\n---*/\nlet x=1;\n",
    )
    .unwrap();
    assert_eq!(
        compose_test262_program(&raw, Test262Variant::RawScript, "HARNESS"),
        "let x=1;\n"
    );
    let strict =
        parse_test262("strict.js", "/*---\nflags: [onlyStrict]\n---*/\nlet x=1;\n").unwrap();
    assert_eq!(
        compose_test262_program(&strict, Test262Variant::StrictScript, "HARNESS"),
        "HARNESS\n'use strict';\nlet x=1;\n"
    );
}

#[test]
fn script_compile_only_detects_parse_errors_without_running_side_effects() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        ctx.compile("globalThis.__ran = true", "parse-only.js", false)
            .unwrap();
        assert!(ctx
            .eval::<bool, _>("globalThis.__ran === undefined")
            .unwrap());
        assert!(ctx.compile("let = ;", "syntax.js", false).is_err());
    });
}

#[test]
fn malformed_metadata_and_unknown_flags_fail_closed() {
    assert!(parse_test262("bad.js", "1 + 1")
        .unwrap_err()
        .to_string()
        .contains("frontmatter"));
    assert!(
        parse_test262("bad.js", "/*---\nflags: [inventedFlag]\n---*/\n1 + 1;\n")
            .unwrap_err()
            .to_string()
            .contains("unknown flag")
    );
}

#[test]
fn exclusion_manifest_requires_accountability_and_rejects_expired_entries() {
    let manifest = ExclusionManifest::new(vec![Exclusion {
        pattern: "built-ins/Atomics/**".into(),
        reason: "agent API is not available in the Rust host".into(),
        owner: "jit-runtime".into(),
        expires: "2099-01-01".into(),
        features: vec!["Atomics".into()],
    }]);
    manifest.validate("2026-08-30").unwrap();

    let expired = ExclusionManifest::new(vec![Exclusion {
        pattern: "**".into(),
        reason: "temporary".into(),
        owner: "nobody".into(),
        expires: "2020-01-01".into(),
        features: vec![],
    }]);
    assert!(expired
        .validate("2026-08-30")
        .unwrap_err()
        .contains("expired"));
}

#[test]
fn suite_discovery_fails_instead_of_reporting_zero_cases() {
    let root = std::env::temp_dir().join(format!("qjsjit-empty-suite-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let error = discover_test262(&root).unwrap_err();
    assert!(error.to_string().contains("zero Test262 cases"));
    fs::remove_dir(root).unwrap();
}

#[test]
fn unknown_features_are_errors_and_explicit_unsupported_features_skip() {
    assert_eq!(
        classify_features(&["BigInt".into()]).unwrap(),
        FeatureDisposition::Supported
    );
    assert!(classify_features(&["future-invention".into()])
        .unwrap_err()
        .contains("unknown Test262 feature"));
    assert_eq!(
        classify_features(&["agent".into()]).unwrap(),
        FeatureDisposition::Unsupported(vec!["agent".into()])
    );
}

#[test]
fn pinned_registry_distinguishes_known_feature_from_typo() {
    let registry = "Symbol.iterator\nIsHTMLDDA\n";
    assert_eq!(
        classify_features_against(&["Symbol.iterator".into()], registry).unwrap(),
        FeatureDisposition::Supported
    );
    assert_eq!(
        classify_features_against(&["IsHTMLDDA".into()], registry).unwrap(),
        FeatureDisposition::Unsupported(vec!["IsHTMLDDA".into()])
    );
    assert!(classify_features_against(&["Symbol.iteratr".into()], registry).is_err());
}

#[test]
fn pinned_quickjs_config_skips_only_explicit_feature_assignments() {
    let registry = "legacy-regexp\nReflect\ncross-realm\n";
    let config = "[features]\nlegacy-regexp=skip\nReflect\ncross-realm\n";
    assert_eq!(
        classify_features_with_config(
            &["legacy-regexp".into(), "Reflect".into()],
            registry,
            config
        )
        .unwrap(),
        FeatureDisposition::Unsupported(vec!["legacy-regexp".into()])
    );
    assert_eq!(
        classify_features_with_config(&["Reflect".into()], registry, config).unwrap(),
        FeatureDisposition::Supported
    );
}

#[test]
fn canonical_observation_preserves_values_json_loses() {
    let source =
        canonical_observation_source("({nan:NaN, negzero:-0, big:2n, hole:[,1], sym:Symbol('x')})");
    assert!(!source.contains("Object.getOwnPropertyDescriptors(value)"));
    assert!(!source.contains("instanceof"));
    assert!(!source.contains("Object.prototype.toString.call(value)"));
    assert!(source.contains("Number.isNaN"));
    assert!(source.contains("Object.is(value, -0)"));
    assert!(source.contains("opaque:true"));
    assert!(source.contains("coverage:'primitive-only'"));
}

#[test]
fn canonical_observation_does_not_trigger_proxy_reflection_or_reorder_events() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let expression = r#"(()=>{globalThis.__events=[];return new Proxy({}, {
            ownKeys(){__events.push('ownKeys');return []},
            getOwnPropertyDescriptor(){__events.push('descriptor');return undefined},
            getPrototypeOf(){__events.push('prototype');return null},
            get(){__events.push('get');return undefined}
        })})()"#;
        let observed: String = ctx.eval(canonical_observation_source(expression)).unwrap();
        assert!(observed.contains("\"opaque\":true"));
        assert_eq!(
            ctx.eval::<String, _>("JSON.stringify(__events)").unwrap(),
            "[]"
        );
    });
}
