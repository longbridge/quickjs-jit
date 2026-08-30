use rquickjs_core::{context::EvalOptions, Context, Runtime};
use rquickjs_jit::{
    correctness::{
        classify_features_with_config, discover_test262, parse_test262, CaseReport, CaseStatus,
        ExclusionManifest, FeatureDisposition, NativeEvidence, NegativePhase, RunMode, SuiteReport,
        Test262Variant,
    },
    JitConfig, JitRuntime,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const TEST262_REVISION: &str = "d5e73fc8d2c663554fb72e2380a8c2bc1a318a33";

struct Options {
    root: PathBuf,
    output: PathBuf,
    mode: RunMode,
    limit: Option<usize>,
    filter: Option<String>,
    shard_index: usize,
    shard_count: usize,
    timeout: Duration,
}

fn options() -> Result<Options, String> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("jit crate has workspace parent");
    let mut root = workspace_root.join("sys/quickjs/test262");
    let mut output = PathBuf::from("target/jit-test262/report.json");
    let mut mode = RunMode::Interpreter;
    let mut limit = None;
    let mut filter = None;
    let mut shard_index = 0;
    let mut shard_count = 1;
    let mut timeout = Duration::from_secs(10);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = |args: &mut std::iter::Skip<std::env::Args>, name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match arg.as_str() {
            "--root" => root = value(&mut args, "--root")?.into(),
            "--output" => output = value(&mut args, "--output")?.into(),
            "--mode" => mode = RunMode::from_str(&value(&mut args, "--mode")?)?,
            "--limit" => {
                limit = Some(
                    value(&mut args, "--limit")?
                        .parse()
                        .map_err(|_| "invalid --limit")?,
                )
            }
            "--filter" => filter = Some(value(&mut args, "--filter")?),
            "--shard-index" => {
                shard_index = value(&mut args, "--shard-index")?
                    .parse()
                    .map_err(|_| "invalid --shard-index")?
            }
            "--shard-count" => {
                shard_count = value(&mut args, "--shard-count")?
                    .parse()
                    .map_err(|_| "invalid --shard-count")?
            }
            "--timeout-ms" => {
                timeout = Duration::from_millis(
                    value(&mut args, "--timeout-ms")?
                        .parse()
                        .map_err(|_| "invalid --timeout-ms")?,
                )
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if shard_count == 0 || shard_index >= shard_count {
        return Err("shard index must be less than nonzero shard count".into());
    }
    Ok(Options {
        root,
        output,
        mode,
        limit,
        filter,
        shard_index,
        shard_count,
        timeout,
    })
}

fn harness(root: &Path, includes: &[String], asynchronous: bool) -> Result<String, String> {
    let mut source = String::new();
    for include in ["sta.js", "assert.js"]
        .into_iter()
        .chain(includes.iter().map(String::as_str))
    {
        let path = root.join("harness").join(include);
        source.push_str(
            &fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?,
        );
        source.push('\n');
    }
    if asynchronous {
        source.push_str("globalThis.__qjsjitDone=false;globalThis.__qjsjitAsyncError=null;globalThis.$DONE=e=>{__qjsjitAsyncError=e==null?null:String(e&&e.stack||e);__qjsjitDone=true};\n");
    }
    Ok(source)
}

fn execute(
    runtime: &Runtime,
    root: &Path,
    source: &str,
    case: &rquickjs_jit::correctness::Test262Case,
    variant: Test262Variant,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let expired = Instant::now() >= deadline;
        if expired {
            flag.store(true, Ordering::SeqCst);
        }
        expired
    })));
    let asynchronous = matches!(
        variant,
        Test262Variant::AsyncScript
            | Test262Variant::StrictAsyncScript
            | Test262Variant::ModuleAsync
            | Test262Variant::StrictModuleAsync
    );
    let mut program = harness(root, case.includes(), asynchronous)?;
    if matches!(
        variant,
        Test262Variant::StrictScript | Test262Variant::StrictAsyncScript
    ) {
        program.push_str("'use strict';\n");
    }
    program.push_str(source);
    let global = !matches!(
        variant,
        Test262Variant::Module
            | Test262Variant::StrictModule
            | Test262Variant::ModuleAsync
            | Test262Variant::StrictModuleAsync
    );
    let context = Context::full(runtime).map_err(|error| format!("context: {error:?}"))?;
    let result = context.with(|ctx| {
        let mut options = EvalOptions::default();
        options.global = global;
        options.strict = matches!(
            variant,
            Test262Variant::StrictScript
                | Test262Variant::StrictAsyncScript
                | Test262Variant::StrictModule
                | Test262Variant::StrictModuleAsync
        );
        options.promise = asynchronous;
        options.filename = Some(case.path().to_owned());
        match ctx.eval_with_options::<(), _>(program.as_str(), options) {
            Ok(()) => Ok(()),
            Err(error) => {
                let caught = ctx.catch();
                let detail = caught.into_object().and_then(|object| {
                    let name = object.get::<_, String>("name").ok()?;
                    let message = object.get::<_, String>("message").ok().unwrap_or_default();
                    Some(format!("{name}: {message}"))
                });
                Err(detail.unwrap_or_else(|| format!("{error:?}")))
            }
        }
    });
    let observed_error = result.err();
    while runtime.is_job_pending() && Instant::now() < deadline {
        if let Err(error) = runtime.execute_pending_job() {
            return Err(format!("pending job: {error:?}"));
        }
    }
    if interrupted.load(Ordering::SeqCst) || Instant::now() >= deadline {
        return Err(format!("timeout after {} ms", timeout.as_millis()));
    }
    if asynchronous {
        let completion = context.with(|ctx| {
            ctx.eval::<String, _>(
                "JSON.stringify([globalThis.__qjsjitDone,globalThis.__qjsjitAsyncError])",
            )
            .map_err(|error| format!("async completion: {error:?}"))
        })?;
        if completion != "[true,null]" {
            return Err(format!(
                "async $DONE did not complete successfully: {completion}"
            ));
        }
    }
    match case.negative() {
        None => observed_error.map_or(Ok(()), Err),
        Some(negative) => match (negative.phase(), observed_error) {
            (
                NegativePhase::Parse | NegativePhase::Resolution | NegativePhase::Runtime,
                Some(error),
            ) if error.contains(negative.error_type()) => Ok(()),
            (_, Some(error)) => Err(format!("expected {}, got {error}", negative.error_type())),
            (_, None) => Err(format!(
                "expected {} {:?} failure",
                negative.error_type(),
                negative.phase()
            )),
        },
    }
}

fn run(options: Options) -> Result<SuiteReport, String> {
    let test_root = options.root.join("test");
    let mut paths = discover_test262(&test_root).map_err(|error| error.to_string())?;
    paths.retain(|path| {
        !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("_FIXTURE"))
    });
    let discovered_files = paths.len();
    if let Some(filter) = &options.filter {
        paths.retain(|path| path.to_string_lossy().contains(filter));
    }
    paths = paths
        .into_iter()
        .enumerate()
        .filter_map(|(index, path)| {
            (index % options.shard_count == options.shard_index).then_some(path)
        })
        .collect();
    if let Some(limit) = options.limit {
        paths.truncate(limit);
    }
    if paths.is_empty() {
        return Err("selected shard contains zero Test262 cases".into());
    }
    let mut reports = Vec::new();
    let feature_registry = fs::read_to_string(options.root.join("features.txt"))
        .map_err(|error| format!("features.txt: {error}"))?;
    let quickjs_config = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("sys/quickjs/test262.conf"),
    )
    .map_err(|error| format!("test262.conf: {error}"))?;
    let exclusions: ExclusionManifest =
        serde_json::from_str(include_str!("../../tests/fixtures/test262-exclusions.json"))
            .map_err(|error| format!("invalid exclusion manifest: {error}"))?;
    exclusions.validate("2026-08-30")?;
    for path in paths {
        let relative = path
            .strip_prefix(&options.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let source =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let case = match parse_test262(&relative, &source) {
            Ok(case) => case,
            Err(error) => {
                reports.push(failure(
                    &relative,
                    Test262Variant::RawScript,
                    error.to_string(),
                ));
                continue;
            }
        };
        let feature_disposition =
            classify_features_with_config(case.features(), &feature_registry, &quickjs_config)?;
        for &variant in case.variants() {
            if let Some(reason) = exclusions.path_reason(&relative) {
                reports.push(CaseReport {
                    path: relative.clone(),
                    variant,
                    status: CaseStatus::Skip,
                    duration_ms: 0,
                    negative_phase: case.negative().map(|n| n.phase()),
                    negative_type: case.negative().map(|n| n.error_type().to_owned()),
                    skip_reason: Some(reason),
                    error: None,
                    native: NativeEvidence {
                        native_entries: 0,
                        tier2_entries: 0,
                        native_exits: 0,
                        unexpected_fallbacks: 0,
                        opcode_ids: vec![],
                        helper_ids: vec![],
                    },
                });
                continue;
            }
            if let FeatureDisposition::Unsupported(features) = &feature_disposition {
                let reason = exclusions
                    .reason_for(&relative, features)
                    .or_else(|| exclusions.reason_for(&relative, &["@quickjs-config-skip".into()]))
                    .ok_or_else(|| {
                        format!(
                            "unsupported features lack checked exclusion: {}",
                            features.join(", ")
                        )
                    })?;
                reports.push(CaseReport {
                    path: relative.clone(),
                    variant,
                    status: CaseStatus::Skip,
                    duration_ms: 0,
                    negative_phase: case.negative().map(|n| n.phase()),
                    negative_type: case.negative().map(|n| n.error_type().to_owned()),
                    skip_reason: Some(reason),
                    error: None,
                    native: NativeEvidence {
                        native_entries: 0,
                        tier2_entries: 0,
                        native_exits: 0,
                        unexpected_fallbacks: 0,
                        opcode_ids: vec![],
                        helper_ids: vec![],
                    },
                });
                continue;
            }
            let started = Instant::now();
            let (result, evidence) = match options.mode {
                RunMode::Interpreter => {
                    let runtime = Runtime::new().map_err(|error| format!("runtime: {error:?}"))?;
                    (
                        execute(
                            &runtime,
                            &options.root,
                            &source,
                            &case,
                            variant,
                            options.timeout,
                        ),
                        NativeEvidence {
                            native_entries: 0,
                            tier2_entries: 0,
                            native_exits: 0,
                            unexpected_fallbacks: 0,
                            opcode_ids: vec![],
                            helper_ids: vec![],
                        },
                    )
                }
                _ => {
                    let config = if options.mode == RunMode::Automatic {
                        JitConfig::default()
                    } else {
                        JitConfig::builder()
                            .call_threshold(1)
                            .loop_threshold(1)
                            .build()
                            .map_err(|e| e.to_string())?
                    };
                    let runtime = JitRuntime::builder()
                        .config(config)
                        .build()
                        .map_err(|error| format!("JIT runtime: {error:?}"))?;
                    let result = execute(
                        &runtime,
                        &options.root,
                        &source,
                        &case,
                        variant,
                        options.timeout,
                    );
                    runtime.jit().poll();
                    let metrics = runtime.metrics();
                    (
                        result,
                        NativeEvidence {
                            native_entries: metrics.native_entries,
                            tier2_entries: metrics.tier2_entries,
                            native_exits: metrics.native_exits,
                            unexpected_fallbacks: metrics.native_fallbacks,
                            opcode_ids: vec![],
                            helper_ids: vec![],
                        },
                    )
                }
            };
            reports.push(CaseReport {
                path: relative.clone(),
                variant,
                status: if result.is_ok() {
                    CaseStatus::Pass
                } else {
                    CaseStatus::Fail
                },
                duration_ms: started.elapsed().as_millis() as u64,
                negative_phase: case.negative().map(|n| n.phase()),
                negative_type: case.negative().map(|n| n.error_type().to_owned()),
                skip_reason: None,
                error: result.err(),
                native: evidence,
            });
        }
    }
    let report = SuiteReport::new("test262-rust-host", TEST262_REVISION, options.mode, reports)
        .with_discovery(discovered_files, options.shard_index, options.shard_count);
    report.validate()?;
    Ok(report)
}

fn failure(path: &str, variant: Test262Variant, error: String) -> CaseReport {
    CaseReport {
        path: path.into(),
        variant,
        status: CaseStatus::Fail,
        duration_ms: 0,
        negative_phase: None,
        negative_type: None,
        skip_reason: None,
        error: Some(error),
        native: NativeEvidence {
            native_entries: 0,
            tier2_entries: 0,
            native_exits: 0,
            unexpected_fallbacks: 0,
            opcode_ids: vec![],
            helper_ids: vec![],
        },
    }
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("jit-test262: {e}");
            return ExitCode::FAILURE;
        }
    };
    let output = options.output.clone();
    match run(options) {
        Ok(report) => {
            if let Some(parent) = output.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("{}: {e}", parent.display());
                    return ExitCode::FAILURE;
                }
            }
            if let Err(e) = fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()) {
                eprintln!("{}: {e}", output.display());
                return ExitCode::FAILURE;
            }
            let failed = report
                .cases
                .iter()
                .filter(|c| matches!(c.status, CaseStatus::Fail))
                .count();
            eprintln!(
                "{}: {} variants, {failed} failed",
                output.display(),
                report.cases.len()
            );
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("jit-test262: {error}");
            ExitCode::FAILURE
        }
    }
}
