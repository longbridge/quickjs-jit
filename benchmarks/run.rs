mod model;

use model::{
    BenchmarkFile, Exclusion, ModeResult, PhaseTiming, Provenance, SampleEvidence, SamplingPolicy,
    WorkloadResult,
};
use rquickjs::{Context, Runtime};
use rquickjs_jit::{abi::AbiInfo, Jit, JitConfig, JitTierPolicy};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const WARMUPS: usize = 5;
const SAMPLES: usize = 30;
const WINDOWS: usize = 10;
const WINDOW: Duration = Duration::from_secs(1);

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
        name: "numeric",
        suite: "rquickjs-jit",
        group: "compute",
        designated: true,
        file: "numeric.js",
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
    pc_entries: u64,
    helper_exits: u64,
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
    for workload in WORKLOADS {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(workload.file);
        for round in 0..WARMUPS {
            for mode in rotated(modes, round) {
                let _ = child(&executable, mode, &path)?;
            }
        }
        let mut samples: BTreeMap<String, Vec<SampleEvidence>> =
            modes.iter().map(|m| (m.clone(), Vec::new())).collect();
        for pair in 0..SAMPLES {
            for mode in rotated(modes, pair) {
                let result = child(&executable, mode, &path)?;
                validate_sample(mode, &result)?;
                samples.get_mut(mode).unwrap().push(evidence(pair, result));
            }
        }
        let mut throughput: BTreeMap<String, Vec<u64>> =
            modes.iter().map(|m| (m.clone(), Vec::new())).collect();
        for window in 0..WINDOWS {
            for mode in rotated(modes, window) {
                let start = Instant::now();
                let mut ops = 0u64;
                while start.elapsed() < WINDOW {
                    validate_sample(mode, &child(&executable, mode, &path)?)?;
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
            builder = builder.tier_policy(JitTierPolicy::BaselineOnly);
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
    let start = Instant::now();
    let _first_checksum = context
        .with(|ctx| ctx.eval::<String, _>("String(workload(2000, 0))"))
        .map_err(err)?;
    phases.first_eval_ns = ns(start.elapsed());
    let threshold_start = Instant::now();
    let mut install_poll = 0u64;
    let mut before = jit.as_ref().map(Jit::metrics).unwrap_or_default();
    for _ in 0..threshold_attempts(mode) {
        let _threshold_checksum: String = context
            .with(|ctx| ctx.eval("String(workload(2000, 0))"))
            .map_err(err)?;
        if let Some(jit) = &jit {
            let poll = Instant::now();
            jit.poll();
            let poll_ns = ns(poll.elapsed());
            let now = jit.metrics();
            if now.installed > before.installed {
                install_poll = install_poll.saturating_add(poll_ns);
            }
            before = now;
            if native_ready(mode, &before) || before.blacklisted > 0 {
                break;
            }
        }
    }
    phases.threshold_crossing_ns = ns(threshold_start.elapsed());
    phases.install_ns = install_poll;
    phases.compile_ns = phases.threshold_crossing_ns.saturating_sub(install_poll);
    let osr_before = before.osr_entries;
    let start = Instant::now();
    let _osr_checksum: String = context
        .with(|ctx| ctx.eval("String(workload(2000, 0))"))
        .map_err(err)?;
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
        checksum = context
            .with(|ctx| ctx.eval("String(workload(2000, 0))"))
            .map_err(err)?;
        if let Some(jit) = &jit {
            jit.poll();
        }
    }
    phases.steady_state_ns = ns(start.elapsed());
    let metrics = jit.as_ref().map(Jit::metrics).unwrap_or_default();
    let abi = AbiInfo::linked().map_err(err)?;
    let result = WorkerResult {
        elapsed_ns: ns(total.elapsed()),
        checksum,
        native_entries: metrics.native_entries,
        native_exits: metrics.native_exits,
        fallbacks: metrics.native_fallbacks,
        retries: metrics.native_retries,
        tier2_entries: metrics.tier2_entries,
        pc_entries: metrics.osr_attempts,
        helper_exits: metrics
            .native_retries
            .saturating_add(metrics.native_fallbacks),
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
        active_ir_bytes: metrics.active_ir_bytes as u64,
        phases,
    };
    println!("{}", serde_json::to_string(&result).map_err(err)?);
    Ok(())
}

fn validate_mode(mode: &str) -> Result<(), String> {
    if matches!(mode, "interpreter" | "tier1" | "tier2" | "automatic") {
        Ok(())
    } else {
        Err(format!("unsupported mode {mode}"))
    }
}
fn native_ready(mode: &str, m: &rquickjs_jit::JitMetrics) -> bool {
    match mode {
        "interpreter" => true,
        "tier1" => m.native_entries > 0,
        "tier2" => m.tier2_entries > 0,
        "automatic" => m.profitability_evaluations > 0 && m.native_entries > 0,
        _ => false,
    }
}
fn threshold_attempts(mode: &str) -> usize {
    if mode == "interpreter" {
        1
    } else {
        64
    }
}
fn validate_sample(mode: &str, r: &WorkerResult) -> Result<(), String> {
    if r.checksum.is_empty() {
        return Err("empty checksum".into());
    }
    if r.native_exits > r.native_entries {
        return Err("native exits exceed entries".into());
    }
    match mode {
        "interpreter" if r.native_entries != 0 => Err("interpreter entered native code".into()),
        "tier1" if r.tier2_entries != 0 => Err("Tier1 policy entered Tier2".into()),
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
        if sample.opcode_fingerprint != first.opcode_fingerprint
            || sample.abi_fingerprint != first.abi_fingerprint
            || sample.config_fingerprint != first.config_fingerprint
        {
            return Err(format!(
                "{mode} fingerprint changed at pair {}",
                sample.pair_index
            ));
        }
    }
    Ok(())
}
fn evidence(pair: usize, r: WorkerResult) -> SampleEvidence {
    SampleEvidence {
        pair_index: pair as u32,
        elapsed_ns: r.elapsed_ns,
        checksum: r.checksum,
        native_entries: r.native_entries,
        native_exits: r.native_exits,
        fallback_count: r.fallbacks,
        retry_count: r.retries,
        tier1_entries: r.native_entries.saturating_sub(r.tier2_entries),
        tier2_entries: r.tier2_entries,
        pc_entries: r.pc_entries,
        helper_exits: r.helper_exits,
        profitability_evaluations: r.profitability_evaluations,
        profitability_approved: r.profitability_approved,
        profitability_rejected: r.profitability_rejected,
        benefit_recordings: r.benefit_recordings,
        measured_benefit_ns: r.measured_benefit_ns,
        opcode_fingerprint: r.opcode_fingerprint,
        abi_fingerprint: r.abi_fingerprint,
        config_fingerprint: r.config_fingerprint,
        peak_rss_bytes: r.peak_rss_bytes,
        code_bytes: r.code_bytes,
        metadata_bytes: r.metadata_bytes,
        peak_compiler_bytes: r.active_ir_bytes,
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
fn write_file(path: &str, modes: Vec<ModeResult>) -> Result<(), String> {
    let file = BenchmarkFile {
        schema: "jit-benchmark-v1".into(),
        provenance: provenance()?,
        policy: SamplingPolicy {
            latency_warmups: WARMUPS as u32,
            latency_processes: SAMPLES as u32,
            throughput_windows: WINDOWS as u32,
            throughput_window_ns: WINDOW.as_nanos() as u64,
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
    Ok(Provenance {
        source_revision: command("git", &["rev-parse", "HEAD"]),
        quickjs_revision: command("git", &["-C", "sys/quickjs", "rev-parse", "HEAD"]),
        source_dirty: !command("git", &["status", "--porcelain"]).is_empty(),
        command: env::args().collect(),
        target: option_env!("TARGET").unwrap_or(env::consts::ARCH).into(),
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
        schema_sha256: sha256("schema/jit-benchmark-v1.json"),
        suites_lock_sha256: sha256("suites.lock"),
    })
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
            pc_entries: 0,
            helper_exits: 0,
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
        assert!(validate_sample("tier1", &r).is_ok());
        r.tier2_entries = 1;
        assert!(validate_sample("tier1", &r).is_err());
        r.native_exits = 2;
        assert!(validate_sample("tier2", &r).is_err());
    }
    #[test]
    fn rotation_interleaves_order() {
        let m = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(rotated(&m, 1).collect::<Vec<_>>(), vec!["b", "c", "a"]);
    }
    #[test]
    fn interpreter_never_pays_a_tier_up_loop() {
        assert_eq!(threshold_attempts("interpreter"), 1);
        assert_eq!(threshold_attempts("automatic"), 64);
    }
}
