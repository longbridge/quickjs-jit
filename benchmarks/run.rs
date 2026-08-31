mod model;

use model::{
    BenchmarkFile, Exclusion, ModeResult, PhaseTiming, Provenance, SampleEvidence, SamplingPolicy,
    WorkloadResult,
};
use rquickjs::{Context, Function, Runtime, Value};
use rquickjs_jit::{abi::AbiInfo, Jit, JitConfig, JitTierPolicy};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const DEFAULT_WARMUPS: usize = 5;
const DEFAULT_SAMPLES: usize = 30;
const DEFAULT_WINDOWS: usize = 10;
const DEFAULT_WINDOW_MS: usize = 1_000;

#[derive(Clone, Copy)]
struct SamplingConfig {
    warmups: usize,
    samples: usize,
    windows: usize,
    window: Duration,
}

#[derive(Clone, Copy)]
struct Workload {
    name: &'static str,
    suite: &'static str,
    group: &'static str,
    designated: bool,
    file: &'static str,
}
const WORKLOADS: &[Workload] = &[
    Workload {
        name: "quickjs-int-arith",
        suite: "QuickJS microbench",
        group: "compute",
        designated: true,
        file: "quickjs-int-arith.js",
    },
    Workload {
        name: "quickjs-bitops",
        suite: "rquickjs-jit local",
        group: "compute",
        designated: false,
        file: "quickjs-bitops.js",
    },
    Workload {
        name: "quickjs-fibonacci",
        suite: "rquickjs-jit local",
        group: "compute",
        designated: false,
        file: "quickjs-fibonacci.js",
    },
    Workload {
        name: "numeric",
        suite: "rquickjs-jit",
        group: "compute",
        designated: true,
        file: "numeric.js",
    },
    Workload {
        name: "scalar-loop",
        suite: "rquickjs-jit focused",
        group: "scalar-loop",
        designated: true,
        file: "scalar-loop.js",
    },
    Workload {
        name: "call-heavy",
        suite: "rquickjs-jit focused",
        group: "call-heavy",
        designated: true,
        file: "call-heavy.js",
    },
    Workload {
        name: "property-heavy",
        suite: "rquickjs-jit focused",
        group: "property-heavy",
        designated: true,
        file: "property-heavy.js",
    },
    Workload {
        name: "fibonacci-iterative",
        suite: "rquickjs-jit focused",
        group: "compute",
        designated: true,
        file: "fibonacci-iterative.js",
    },
    Workload {
        name: "fibonacci-recursive",
        suite: "rquickjs-jit focused",
        group: "call-heavy",
        designated: false,
        file: "fibonacci-recursive.js",
    },
    Workload {
        name: "collections",
        suite: "rquickjs-jit",
        group: "compute",
        designated: false,
        file: "collections.js",
    },
    Workload {
        name: "strings-json",
        suite: "rquickjs-jit",
        group: "compute",
        designated: false,
        file: "strings-json.js",
    },
    Workload {
        name: "calls-closures",
        suite: "rquickjs-jit",
        group: "compute",
        designated: false,
        file: "calls-closures.js",
    },
    Workload {
        name: "adversarial",
        suite: "rquickjs-jit",
        group: "adversarial",
        designated: false,
        file: "adversarial.js",
    },
    Workload {
        name: "float64-dense",
        suite: "rquickjs-jit matrix",
        group: "float64",
        designated: false,
        file: "float64-dense.js",
    },
    Workload {
        name: "strings-regexp",
        suite: "rquickjs-jit matrix",
        group: "strings-regexp",
        designated: false,
        file: "strings-regexp.js",
    },
    Workload {
        name: "arrays-typed",
        suite: "rquickjs-jit matrix",
        group: "arrays-typed",
        designated: false,
        file: "arrays-typed.js",
    },
    Workload {
        name: "objects-polymorphic",
        suite: "rquickjs-jit matrix",
        group: "objects-polymorphic",
        designated: false,
        file: "objects-polymorphic.js",
    },
    Workload {
        name: "calls-recursion-closures",
        suite: "rquickjs-jit matrix",
        group: "calls-recursion-closures",
        designated: false,
        file: "calls-recursion-closures.js",
    },
    Workload {
        name: "json-codec",
        suite: "rquickjs-jit matrix",
        group: "json-codec",
        designated: false,
        file: "json-codec.js",
    },
    Workload {
        name: "map-set-bigint",
        suite: "rquickjs-jit matrix",
        group: "map-set-bigint",
        designated: false,
        file: "map-set-bigint.js",
    },
    Workload {
        name: "exceptions-promises-async",
        suite: "rquickjs-jit matrix",
        group: "exceptions-promises-async",
        designated: false,
        file: "exceptions-promises-async.js",
    },
];

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WorkerResult {
    elapsed_ns: u64,
    checksum: String,
    native_entries: u64,
    native_exits: u64,
    fallbacks: u64,
    retries: u64,
    tier2_entries: u64,
    deopts: u64,
    osr_attempts: u64,
    profitability_evaluations: u64,
    profitability_approved: u64,
    profitability_rejected: u64,
    benefit_recordings: u64,
    measured_benefit_ns: u64,
    opcode_fingerprint: u64,
    abi_fingerprint: u64,
    config_fingerprint: u64,
    peak_rss_bytes: u64,
    code_bytes: u64,
    metadata_bytes: u64,
    active_ir_bytes: u64,
    phases: PhaseTiming,
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn real_main() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("worker") => worker(&value(&args,"--mode")?, &value(&args,"--script")?),
        Some("run") => { let mode=value(&args,"--mode")?; write_file(&value(&args,"--output")?, measure_interleaved(&[mode])?) }
        Some("compare") => {
            let modes=value(&args,"--modes")?.split(',').map(str::to_owned).collect::<Vec<_>>();
            let output=value(&args,"--output")?; write_file(&output,measure_interleaved(&modes)?)?;
            if let Ok(report)=value(&args,"--report") { let status=Command::new(sibling_reporter()?).args(["--input",&output,"--output",&report]).status().map_err(err)?; if !status.success(){return Err("performance gates failed; report contains evidence".into())} }
            Ok(())
        }
        _ => Err("usage: jit-bench run --mode MODE --output FILE | compare --modes ... --output FILE --report FILE".into()),
    }
}

fn measure_interleaved(modes: &[String]) -> Result<Vec<ModeResult>, String> {
    for mode in modes {
        validate_mode(mode)?;
    }
    let executable = env::current_exe().map_err(err)?;
    let mut by_mode: BTreeMap<String, Vec<WorkloadResult>> =
        modes.iter().map(|m| (m.clone(), Vec::new())).collect();
    let sampling = sampling_config()?;
    let workloads = selected_workloads()?;
    for workload in workloads {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(workload.file);
        for round in 0..sampling.warmups {
            for mode in rotated(modes, round) {
                let _ = child(&executable, mode, &path)?;
            }
        }
        let mut samples: BTreeMap<String, Vec<SampleEvidence>> =
            modes.iter().map(|m| (m.clone(), Vec::new())).collect();
        for pair in 0..sampling.samples {
            for mode in rotated(modes, pair) {
                let result = child(&executable, mode, &path)?;
                validate_sample(mode, &result, workload.designated)?;
                samples
                    .get_mut(mode)
                    .unwrap()
                    .push(evidence_for_mode(mode, pair, result));
            }
        }
        let mut throughput: BTreeMap<String, Vec<u64>> =
            modes.iter().map(|m| (m.clone(), Vec::new())).collect();
        for window in 0..sampling.windows {
            for mode in rotated(modes, window) {
                let start = Instant::now();
                let mut ops = 0u64;
                while start.elapsed() < sampling.window {
                    validate_sample(mode, &child(&executable, mode, &path)?, workload.designated)?;
                    ops = ops.saturating_add(1);
                }
                throughput.get_mut(mode).unwrap().push(ops);
            }
        }
        for mode in modes {
            let mode_samples = samples.remove(mode).unwrap();
            ensure_consistent_samples(mode, &mode_samples)?;
            let raw = mode_samples
                .iter()
                .map(|s| s.elapsed_ns)
                .collect::<Vec<_>>();
            let (median, mad, p95, p99, ci) = model::summarize(raw.clone());
            let compile_ns = median_field(&mode_samples, |s| s.phases.compile_ns);
            let install_ns = median_field(&mode_samples, |s| s.phases.install_ns);
            by_mode.get_mut(mode).unwrap().push(WorkloadResult {
                name: workload.name.into(),
                suite: workload.suite.into(),
                group: workload.group.into(),
                designated_kernel: workload.designated,
                samples: mode_samples,
                raw_latency_ns: raw,
                raw_throughput_ops: throughput.remove(mode).unwrap(),
                median_ns: median,
                mad_ns: mad,
                p95_ns: p95,
                p99_ns: p99,
                ci95_ns: ci,
                compile_ns,
                install_ns,
                break_even_executions: None,
            });
        }
    }
    let mut results = by_mode
        .into_iter()
        .map(|(mode, workloads)| ModeResult { mode, workloads })
        .collect::<Vec<_>>();
    fill_break_even(&mut results);
    Ok(results)
}

fn worker(mode: &str, script: &str) -> Result<(), String> {
    validate_mode(mode)?;
    let total = Instant::now();
    let source = fs::read(script).map_err(err)?;
    let mut phases = PhaseTiming::default();
    let start = Instant::now();
    let runtime = Runtime::new().map_err(err)?;
    phases.runtime_create_ns = ns(start.elapsed());
    let start = Instant::now();
    let (jit, config_fingerprint) = if mode == "interpreter" {
        (None, hash64(b"interpreter"))
    } else {
        let mut builder = JitConfig::builder();
        if matches!(mode, "tier1" | "tier2") {
            builder = builder.call_threshold(1).loop_threshold(1);
        }
        if mode == "tier1" {
            // The second entry freezes the first complete argument/return
            // profile, allowing baseline artifacts to publish a bounded
            // scalar entry for baseline-to-baseline calls.
            builder = builder
                .call_threshold(2)
                .tier_policy(JitTierPolicy::BaselineOnly);
        }
        if mode == "tier2" {
            builder = builder
                .tier_policy(JitTierPolicy::Optimize)
                .force_optimized_for_test(true);
        }
        let config = builder.build().map_err(err)?;
        let fingerprint = hash64(format!("{config:?}").as_bytes());
        (
            Some(Jit::attach(&runtime, config).map_err(err)?),
            fingerprint,
        )
    };
    phases.jit_attach_ns = ns(start.elapsed());
    let start = Instant::now();
    let context = Context::full(&runtime).map_err(err)?;
    phases.context_create_ns = ns(start.elapsed());
    let start = Instant::now();
    context
        .with(|ctx| ctx.eval::<(), _>(source.as_slice()))
        .map_err(err)?;
    phases.definition_eval_ns = ns(start.elapsed());
    let tier1_ready_installs = context
        .with(|ctx| {
            ctx.eval::<u64, _>(
                "typeof globalThis.tier1ReadyInstalls === 'number' ? globalThis.tier1ReadyInstalls : 1",
            )
        })
        .map_err(err)?;
    let start = Instant::now();
    let _first_checksum = invoke_workload(&context)?;
    phases.first_eval_ns = ns(start.elapsed());
    let threshold_start = Instant::now();
    let threshold_deadline = threshold_start + threshold_timeout(mode);
    let mut install_poll = 0u64;
    let mut before = jit.as_ref().map(Jit::metrics).unwrap_or_default();
    loop {
        let _threshold_checksum = invoke_workload(&context)?;
        if let Some(jit) = &jit {
            let poll = Instant::now();
            jit.poll();
            let poll_ns = ns(poll.elapsed());
            let now = jit.metrics();
            if now.installed > before.installed {
                install_poll = install_poll.saturating_add(poll_ns);
            }
            before = now;
            if native_ready(mode, &before, tier1_ready_installs) || before.blacklisted > 0 {
                break;
            }
            if Instant::now() >= threshold_deadline {
                break;
            }
            std::thread::sleep(Duration::from_micros(50));
        } else {
            break;
        }
    }
    phases.threshold_crossing_ns = ns(threshold_start.elapsed());
    phases.compile_ns = before.compile_ns;
    phases.install_ns = before.install_ns.max(install_poll);
    let osr_before = before.osr_entries;
    let start = Instant::now();
    let _osr_checksum = invoke_workload(&context)?;
    if let Some(jit) = &jit {
        jit.poll();
    }
    phases.osr_ns = if jit
        .as_ref()
        .is_some_and(|j| j.metrics().osr_entries > osr_before)
    {
        ns(start.elapsed())
    } else {
        0
    };
    let start = Instant::now();
    let mut checksum = String::new();
    for _ in 0..10 {
        checksum = invoke_workload(&context)?;
        if let Some(jit) = &jit {
            jit.poll();
        }
    }
    phases.steady_state_ns = ns(start.elapsed());
    let metrics = jit.as_ref().map(Jit::metrics).unwrap_or_default();
    let abi = AbiInfo::linked().map_err(err)?;
    phases.total_ns = ns(total.elapsed());
    let result = WorkerResult {
        elapsed_ns: phases.steady_state_ns,
        checksum,
        native_entries: metrics.native_entries,
        native_exits: metrics.native_exits,
        fallbacks: metrics.native_fallbacks,
        retries: metrics.native_retries,
        tier2_entries: metrics.tier2_entries,
        deopts: metrics.deopts,
        osr_attempts: metrics.osr_attempts,
        profitability_evaluations: metrics.profitability_evaluations,
        profitability_approved: metrics.profitability_approved,
        profitability_rejected: metrics.profitability_rejected,
        benefit_recordings: metrics.benefit_recordings,
        measured_benefit_ns: metrics.measured_benefit_ns,
        opcode_fingerprint: abi.opcode_fingerprint(),
        abi_fingerprint: abi.source_revision() ^ abi.build_fingerprint(),
        config_fingerprint,
        peak_rss_bytes: worker_peak_rss(),
        code_bytes: metrics.code_bytes as u64,
        metadata_bytes: metrics.metadata_bytes as u64,
        active_ir_bytes: metrics.peak_compiler_bytes as u64,
        phases,
    };
    println!("{}", serde_json::to_string(&result).map_err(err)?);
    Ok(())
}

fn invoke_workload(context: &Context) -> Result<String, String> {
    context
        .with(|ctx| {
            let workload: Function = ctx.globals().get("workload")?;
            // A workload may expose a stable callable/object input without
            // resolving it through a global from inside the measured
            // function. This keeps the benchmark honest about call/property
            // bytecodes while making the function independently tierable.
            let argument: Value = ctx.globals().get("workloadArgument")?;
            let result: Value = if argument.is_undefined() {
                workload.call((2_000, 0))?
            } else {
                workload.call((2_000, 0, argument))?
            };
            let result = if let Some(promise) = result.as_promise() {
                promise.finish::<Value>()?
            } else {
                result
            };
            if let Some(number) = result.as_number() {
                Ok(format!("number:{:016x}", number.to_bits()))
            } else if let Some(string) = result.as_string() {
                Ok(format!("string:{}", string.to_string()?))
            } else if let Some(boolean) = result.as_bool() {
                Ok(format!("boolean:{boolean}"))
            } else if result.is_null() {
                Ok("null".into())
            } else if result.is_undefined() {
                Ok("undefined".into())
            } else {
                Err(rquickjs::Error::new_from_js(
                    result.type_name(),
                    "benchmark checksum primitive",
                ))
            }
        })
        .map_err(err)
}

fn validate_mode(mode: &str) -> Result<(), String> {
    if matches!(
        mode,
        "interpreter" | "tier1" | "tier2" | "automatic" | "bun"
    ) {
        Ok(())
    } else {
        Err(format!("unsupported mode {mode}"))
    }
}

fn sampling_config() -> Result<SamplingConfig, String> {
    Ok(SamplingConfig {
        warmups: positive_env("JIT_BENCH_WARMUPS", DEFAULT_WARMUPS)?,
        samples: positive_env("JIT_BENCH_SAMPLES", DEFAULT_SAMPLES)?,
        windows: positive_env("JIT_BENCH_WINDOWS", DEFAULT_WINDOWS)?,
        window: Duration::from_millis(
            positive_env("JIT_BENCH_WINDOW_MS", DEFAULT_WINDOW_MS)? as u64
        ),
    })
}

fn positive_env(name: &str, default: usize) -> Result<usize, String> {
    match env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|_| format!("{name} must be a positive integer"))
            .and_then(|value| {
                (value > 0)
                    .then_some(value)
                    .ok_or_else(|| format!("{name} must be greater than zero"))
            }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("cannot read {name}: {error}")),
    }
}

fn selected_workloads() -> Result<Vec<&'static Workload>, String> {
    let Ok(filter) = env::var("JIT_BENCH_WORKLOADS") else {
        return Ok(WORKLOADS.iter().collect());
    };
    let names = filter
        .split(',')
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err("JIT_BENCH_WORKLOADS must name at least one workload".into());
    }
    let selected = WORKLOADS
        .iter()
        .filter(|workload| names.contains(&workload.name))
        .collect::<Vec<_>>();
    if selected.len() != names.len() {
        let unknown = names
            .iter()
            .filter(|name| !WORKLOADS.iter().any(|workload| workload.name == **name))
            .copied()
            .collect::<Vec<_>>();
        return Err(format!(
            "unknown benchmark workload(s): {}",
            unknown.join(",")
        ));
    }
    Ok(selected)
}
fn native_ready(mode: &str, m: &rquickjs_jit::JitMetrics, tier1_ready_installs: u64) -> bool {
    match mode {
        "interpreter" => true,
        "tier1" => m.native_entries > 0 && m.installed >= tier1_ready_installs,
        "tier2" => m.tier2_entries > 0,
        "automatic" => m.profitability_approved > 0 && m.tier2_entries > 0,
        _ => false,
    }
}
fn threshold_timeout(mode: &str) -> Duration {
    if mode == "interpreter" {
        Duration::ZERO
    } else {
        Duration::from_secs(2)
    }
}
fn validate_sample(mode: &str, r: &WorkerResult, requires_native: bool) -> Result<(), String> {
    if r.checksum.is_empty() {
        return Err("empty checksum".into());
    }
    if mode != "bun" && r.native_exits > r.native_entries {
        return Err("native exits exceed entries".into());
    }
    match mode {
        "interpreter" if r.native_entries != 0 => Err("interpreter entered native code".into()),
        "tier1" if requires_native && r.native_entries == 0 => {
            Err("Tier1 sample never entered native code".into())
        }
        "tier1" if r.tier2_entries != 0 => Err("Tier1 policy entered Tier2".into()),
        "tier2" if requires_native && r.tier2_entries == 0 => {
            Err("Tier2 sample never entered Tier2 code".into())
        }
        _ => Ok(()),
    }
}
fn ensure_consistent_samples(mode: &str, samples: &[SampleEvidence]) -> Result<(), String> {
    let first = samples.first().ok_or("no samples")?;
    for sample in samples {
        if sample.checksum != first.checksum {
            return Err(format!(
                "{mode} checksum changed at pair {}",
                sample.pair_index
            ));
        }
        if mode != "bun"
            && (sample.opcode_fingerprint != first.opcode_fingerprint
                || sample.abi_fingerprint != first.abi_fingerprint
                || sample.config_fingerprint != first.config_fingerprint)
        {
            return Err(format!(
                "{mode} fingerprint changed at pair {}",
                sample.pair_index
            ));
        }
    }
    Ok(())
}
fn evidence_for_mode(mode: &str, pair: usize, r: WorkerResult) -> SampleEvidence {
    let external = mode == "bun";
    SampleEvidence {
        pair_index: pair as u32,
        elapsed_ns: r.elapsed_ns,
        checksum: r.checksum,
        native_entries: (!external).then_some(r.native_entries),
        native_exits: (!external).then_some(r.native_exits),
        fallback_count: (!external).then_some(r.fallbacks),
        retry_count: (!external).then_some(r.retries),
        tier1_entries: (!external).then_some(r.native_entries.saturating_sub(r.tier2_entries)),
        tier2_entries: (!external).then_some(r.tier2_entries),
        deopt_count: (!external).then_some(r.deopts),
        osr_attempts: (!external).then_some(r.osr_attempts),
        profitability_evaluations: (!external).then_some(r.profitability_evaluations),
        profitability_approved: (!external).then_some(r.profitability_approved),
        profitability_rejected: (!external).then_some(r.profitability_rejected),
        benefit_recordings: (!external).then_some(r.benefit_recordings),
        measured_benefit_ns: (!external).then_some(r.measured_benefit_ns),
        opcode_fingerprint: (!external).then_some(r.opcode_fingerprint),
        abi_fingerprint: (!external).then_some(r.abi_fingerprint),
        config_fingerprint: (!external).then_some(r.config_fingerprint),
        peak_rss_bytes: (!external).then_some(r.peak_rss_bytes),
        code_bytes: (!external).then_some(r.code_bytes),
        metadata_bytes: (!external).then_some(r.metadata_bytes),
        peak_compiler_bytes: (!external).then_some(r.active_ir_bytes),
        phases: r.phases,
    }
}
fn rotated<'a>(modes: &'a [String], round: usize) -> impl Iterator<Item = &'a str> + 'a {
    (0..modes.len()).map(move |i| modes[(i + round) % modes.len()].as_str())
}
fn median_field(samples: &[SampleEvidence], field: impl Fn(&SampleEvidence) -> u64) -> u64 {
    let mut v = samples.iter().map(field).collect::<Vec<_>>();
    v.sort_unstable();
    model::quantile(&v, 0.5)
}
fn fill_break_even(results: &mut [ModeResult]) {
    let Some(base) = results.iter().find(|m| m.mode == "interpreter").cloned() else {
        return;
    };
    for mode in results.iter_mut().filter(|m| m.mode != "interpreter") {
        for w in &mut mode.workloads {
            if let Some(b) = base.workloads.iter().find(|b| b.name == w.name) {
                let saved = b.median_ns.saturating_sub(w.median_ns);
                w.break_even_executions =
                    (saved > 0).then(|| w.compile_ns.saturating_add(w.install_ns).div_ceil(saved));
            }
        }
    }
}
fn child(executable: &Path, mode: &str, script: &Path) -> Result<WorkerResult, String> {
    if mode == "bun" {
        return bun_child(script);
    }
    let output = Command::new(executable)
        .args(["worker", "--mode", mode, "--script"])
        .arg(script)
        .output()
        .map_err(err)?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(err)
}
fn bun_child(script: &Path) -> Result<WorkerResult, String> {
    let bun = env::var("JIT_BENCH_BUN").unwrap_or_else(|_| "bun".into());
    let wrapper = r#"
import { readFileSync } from 'node:fs';
(0,eval)(readFileSync(process.argv[1],'utf8'));
const first=workload(2000,0,globalThis.workloadArgument); if(first&&typeof first.then==='function') await first;
const start=Bun.nanoseconds(); let result;
for(let i=0;i<10;i++){result=workload(2000,0,globalThis.workloadArgument);if(result&&typeof result.then==='function')result=await result}
const elapsed=Bun.nanoseconds()-start;
function checksum(v){if(typeof v==='number'){const b=new ArrayBuffer(8),d=new DataView(b);d.setFloat64(0,v,false);return 'number:'+d.getBigUint64(0,false).toString(16).padStart(16,'0')}if(typeof v==='string')return 'string:'+v;if(typeof v==='boolean')return 'boolean:'+v;if(v===null)return 'null';if(v===undefined)return 'undefined';throw new Error('checksum primitive required')}
console.log(JSON.stringify({elapsed_ns:elapsed,checksum:checksum(result)}));
"#;
    let started = Instant::now();
    let output = Command::new(&bun)
        .args(["--smol", "-e", wrapper])
        .arg(script)
        .output()
        .map_err(err)?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    #[derive(Deserialize)]
    struct BunOut {
        elapsed_ns: u64,
        checksum: String,
    }
    let out: BunOut = serde_json::from_slice(&output.stdout).map_err(err)?;
    let phases = PhaseTiming {
        steady_state_ns: out.elapsed_ns,
        total_ns: ns(started.elapsed()),
        ..Default::default()
    };
    Ok(WorkerResult {
        elapsed_ns: out.elapsed_ns,
        checksum: out.checksum,
        native_entries: 0,
        native_exits: 0,
        fallbacks: 0,
        retries: 0,
        tier2_entries: 0,
        deopts: 0,
        osr_attempts: 0,
        profitability_evaluations: 0,
        profitability_approved: 0,
        profitability_rejected: 0,
        benefit_recordings: 0,
        measured_benefit_ns: 0,
        opcode_fingerprint: 0,
        abi_fingerprint: 0,
        config_fingerprint: hash64(format!("{}:{}", command(&bun, &["--version"]), bun).as_bytes()),
        peak_rss_bytes: 0,
        code_bytes: 0,
        metadata_bytes: 0,
        active_ir_bytes: 0,
        phases,
    })
}
fn write_file(path: &str, modes: Vec<ModeResult>) -> Result<(), String> {
    let mut mode_names = modes.iter().map(|m| m.mode.as_str()).collect::<Vec<_>>();
    mode_names.sort_unstable();
    if mode_names != ["automatic", "interpreter", "tier1", "tier2"]
        && mode_names != ["automatic", "bun", "interpreter", "tier1", "tier2"]
    {
        return Err("benchmark evidence requires the four rquickjs modes and optional bun".into());
    }
    let sampling = sampling_config()?;
    let file = BenchmarkFile {
        schema: "jit-benchmark-v1".into(),
        provenance: provenance()?,
        policy: SamplingPolicy {
            latency_warmups: sampling.warmups as u32,
            latency_processes: sampling.samples as u32,
            throughput_windows: sampling.windows as u32,
            throughput_window_ns: sampling.window.as_nanos() as u64,
            bootstrap_resamples: 10_000,
            pairing:
                "round-robin interleaved fresh processes; pair_index is the joint resampling unit"
                    .into(),
        },
        modes,
        exclusions: vec![
            Exclusion {
                suite: "SunSpider".into(),
                test: "all".into(),
                reason: "not vendored; no redistribution/import performed".into(),
            },
            Exclusion {
                suite: "JetStream".into(),
                test: "all".into(),
                reason: "not vendored; no runnable components available locally".into(),
            },
        ],
    };
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).map_err(err)?
    }
    fs::write(path, serde_json::to_vec_pretty(&file).map_err(err)?).map_err(err)
}
fn provenance() -> Result<Provenance, String> {
    let (stripped_no_jit_bytes, stripped_jit_bytes) = stripped_probe_sizes()?;
    let bun_path = command("sh", &["-c", "command -v bun"]);
    let bun_available = !bun_path.is_empty();
    Ok(Provenance {
        source_revision: command("git", &["rev-parse", "HEAD"]),
        quickjs_revision: command("git", &["-C", "sys/quickjs", "rev-parse", "HEAD"]),
        source_dirty: !command("git", &["status", "--porcelain"]).is_empty(),
        command: env::args().collect(),
        target: option_env!("TARGET").unwrap_or(env::consts::ARCH).into(),
        target_triple: rustc_host_triple(),
        os: env::consts::OS.into(),
        kernel: command("uname", &["-sr"]),
        cpu: fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|x| x.starts_with("model name"))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "unknown".into()),
        power_mode: fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .into(),
        rustc: command("rustc", &["-Vv"]),
        llvm: command("rustc", &["-vV"]),
        executable_bytes: fs::metadata(env::current_exe().map_err(err)?)
            .map_err(err)?
            .len(),
        stripped_jit_bytes,
        stripped_no_jit_bytes,
        stripped_jit_delta_bytes: i64::try_from(stripped_jit_bytes)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(stripped_no_jit_bytes).unwrap_or(i64::MAX)),
        schema_sha256: sha256("schema/jit-benchmark-v1.json"),
        suites_lock_sha256: sha256("suites.lock"),
        bun_version: bun_available.then(|| command(&bun_path, &["--version"])),
        bun_sha256: bun_available.then(|| sha256_file(&bun_path)),
        bun_path: bun_available.then_some(bun_path),
    })
}
fn rustc_host_triple() -> String {
    command("rustc", &["-vV"])
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown-unknown-unknown")
        .to_owned()
}
fn stripped_probe_sizes() -> Result<(u64, u64), String> {
    let exe = env::current_exe().map_err(err)?;
    let dir = exe.parent().ok_or("benchmark executable has no parent")?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let no_jit = dir.join(format!("jit-size-no-jit{suffix}"));
    let jit = dir.join(format!("jit-size-jit{suffix}"));
    if !no_jit.is_file() || !jit.is_file() {
        return Err("build --release --bins first: stripped size probes are missing".into());
    }
    Ok((stripped_size(&no_jit)?, stripped_size(&jit)?))
}
fn stripped_size(path: &Path) -> Result<u64, String> {
    let temp = env::temp_dir().join(format!("rquickjs-jit-size-{}", std::process::id()));
    fs::copy(path, &temp).map_err(err)?;
    let status = Command::new("strip").arg(&temp).status().map_err(err)?;
    if !status.success() {
        return Err("strip failed for binary size probe".into());
    }
    let size = fs::metadata(&temp).map_err(err)?.len();
    let _ = fs::remove_file(temp);
    Ok(size)
}
fn worker_peak_rss() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
        .saturating_mul(1024)
}
fn sha256(relative: &str) -> String {
    command(
        "sha256sum",
        &[&Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(relative)
            .to_string_lossy()],
    )
    .split_whitespace()
    .next()
    .unwrap_or("unknown")
    .into()
}
fn sha256_file(path: &str) -> String {
    command("sha256sum", &[path])
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .into()
}
fn hash64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn ns(d: Duration) -> u64 {
    d.as_nanos().try_into().unwrap_or(u64::MAX)
}
fn command(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}
fn sibling_reporter() -> Result<PathBuf, String> {
    let mut p = env::current_exe().map_err(err)?;
    p.set_file_name(if cfg!(windows) {
        "jit-bench-report.exe"
    } else {
        "jit-bench-report"
    });
    Ok(p)
}
fn value(args: &[String], flag: &str) -> Result<String, String> {
    args.iter()
        .position(|x| x == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .ok_or_else(|| format!("missing {flag}"))
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_rejects_cross_tier_and_impossible_counters() {
        let mut r = WorkerResult {
            elapsed_ns: 1,
            checksum: "x".into(),
            native_entries: 0,
            native_exits: 0,
            fallbacks: 0,
            retries: 0,
            tier2_entries: 0,
            deopts: 0,
            osr_attempts: 0,
            profitability_evaluations: 0,
            profitability_approved: 0,
            profitability_rejected: 0,
            benefit_recordings: 0,
            measured_benefit_ns: 0,
            opcode_fingerprint: 1,
            abi_fingerprint: 1,
            config_fingerprint: 1,
            peak_rss_bytes: 1,
            code_bytes: 0,
            metadata_bytes: 0,
            active_ir_bytes: 0,
            phases: PhaseTiming::default(),
        };
        assert!(validate_sample("tier1", &r, true).is_err());
        assert!(validate_sample("tier1", &r, false).is_ok());
        r.native_entries = 1;
        assert!(validate_sample("tier1", &r, true).is_ok());
        r.tier2_entries = 1;
        assert!(validate_sample("tier1", &r, true).is_err());
        assert!(validate_sample("tier2", &r, true).is_ok());
        r.native_exits = 2;
        assert!(validate_sample("tier2", &r, true).is_err());
    }
    #[test]
    fn rotation_interleaves_order() {
        let m = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(rotated(&m, 1).collect::<Vec<_>>(), vec!["b", "c", "a"]);
    }
    #[test]
    fn interpreter_never_pays_a_tier_up_loop() {
        assert_eq!(threshold_timeout("interpreter"), Duration::ZERO);
        assert_eq!(threshold_timeout("automatic"), Duration::from_secs(2));
    }
    #[test]
    fn automatic_warmup_waits_for_an_installed_profitable_tier2() {
        let mut metrics = rquickjs_jit::JitMetrics::default();
        metrics.native_entries = 8;
        metrics.profitability_evaluations = 1;
        assert!(!native_ready("automatic", &metrics, 1));
        metrics.profitability_approved = 1;
        assert!(!native_ready("automatic", &metrics, 1));
        metrics.tier2_entries = 1;
        assert!(native_ready("automatic", &metrics, 1));
    }
    #[test]
    fn focused_performance_categories_are_present_and_designated() {
        for (name, group) in [
            ("scalar-loop", "scalar-loop"),
            ("call-heavy", "call-heavy"),
            ("property-heavy", "property-heavy"),
        ] {
            let workload = WORKLOADS
                .iter()
                .find(|workload| workload.name == name)
                .unwrap();
            assert_eq!(workload.group, group);
            assert!(workload.designated);
            assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(workload.file)
                .is_file());
        }
    }

    #[test]
    fn bun_external_samples_use_null_native_counters_and_matching_checksums() {
        let bun = env::var("JIT_BENCH_BUN").unwrap_or_else(|_| "bun".into());
        if !Command::new(&bun)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }
        for script in ["scalar-loop.js", "numeric.js", "quickjs-fibonacci.js"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(script);
            let result = bun_child(&path).unwrap();
            validate_sample("bun", &result, true).unwrap();
            let sample = evidence_for_mode("bun", 0, result);
            assert!(sample.native_entries.is_none() && sample.native_exits.is_none());
            assert!(sample.tier1_entries.is_none() && sample.tier2_entries.is_none());
            assert!(!sample.checksum.is_empty());
        }
    }
    #[test]
    fn recursive_fibonacci_is_a_non_designated_call_path_probe() {
        let iterative = WORKLOADS
            .iter()
            .find(|workload| workload.name == "fibonacci-iterative")
            .unwrap();
        assert_eq!(iterative.group, "compute");
        assert!(iterative.designated);

        let recursive = WORKLOADS
            .iter()
            .find(|workload| workload.name == "fibonacci-recursive")
            .unwrap();
        assert_eq!(recursive.group, "call-heavy");
        assert!(!recursive.designated);
    }

    #[test]
    fn expanded_performance_matrix_is_complete_and_explicitly_non_designated() {
        for (name, group) in [
            ("float64-dense", "float64"),
            ("strings-regexp", "strings-regexp"),
            ("arrays-typed", "arrays-typed"),
            ("objects-polymorphic", "objects-polymorphic"),
            ("calls-recursion-closures", "calls-recursion-closures"),
            ("json-codec", "json-codec"),
            ("map-set-bigint", "map-set-bigint"),
            ("exceptions-promises-async", "exceptions-promises-async"),
        ] {
            let workload = WORKLOADS.iter().find(|w| w.name == name).unwrap();
            assert_eq!(workload.group, group);
            assert!(
                !workload.designated,
                "coverage is evidence, not a native-entry claim"
            );
            assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(workload.file)
                .is_file());
        }
    }

    #[test]
    fn expanded_matrix_has_repeatable_primitive_checksums_in_quickjs() {
        for workload in WORKLOADS
            .iter()
            .filter(|w| w.suite == "rquickjs-jit matrix")
        {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            let source = fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts")
                    .join(workload.file),
            )
            .unwrap();
            context
                .with(|ctx| ctx.eval::<(), _>(source.as_slice()))
                .unwrap();
            let first = invoke_workload(&context).unwrap();
            let second = invoke_workload(&context).unwrap();
            assert_eq!(first, second, "{} checksum drifted", workload.name);
            assert!(
                first.starts_with("number:") || first.starts_with("string:"),
                "{} did not return a reportable primitive checksum",
                workload.name
            );
        }
    }

    #[cfg(all(
        target_os = "linux",
        target_endian = "little",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    #[test]
    fn harness_omits_the_optional_argument_for_two_parameter_numeric_kernels() {
        let runtime = Runtime::new().unwrap();
        let jit = Jit::attach(
            &runtime,
            JitConfig::builder()
                .call_threshold(1)
                .loop_threshold(1)
                .tier_policy(JitTierPolicy::Optimize)
                .force_optimized_for_test(true)
                .build()
                .unwrap(),
        )
        .unwrap();
        let context = Context::full(&runtime).unwrap();
        let source = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join("scalar-loop.js"),
        )
        .unwrap();
        context
            .with(|ctx| ctx.eval::<(), _>(source.as_slice()))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            invoke_workload(&context).unwrap();
            jit.poll();
            if jit.metrics().tier2_entries > 0 {
                return;
            }
            std::thread::sleep(Duration::from_micros(50));
        }
        panic!(
            "the harness changed the two-argument kernel signature: {:?}",
            jit.metrics()
        );
    }
}
