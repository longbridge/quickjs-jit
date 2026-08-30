mod model;

use model::{BenchmarkFile, Exclusion, ModeResult, Provenance, SamplingPolicy, WorkloadResult};
use rquickjs::{Context, Runtime};
use rquickjs_jit::{Jit, JitConfig, JitTierPolicy};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const WARMUPS: usize = 5;
const SAMPLES: usize = 30;
const WINDOWS: usize = 10;
const WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WorkerResult {
    elapsed_ns: u64,
    checksum: String,
    native_entries: u64,
    native_exits: u64,
    fallbacks: u64,
    retries: u64,
    tier2_entries: u64,
    osr_entries: u64,
    code_bytes: u64,
    metadata_bytes: u64,
    active_ir_bytes: u64,
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
        Some("worker") => worker(&value(&args, "--mode")?, &value(&args, "--script")?),
        Some("run") => {
            let mode = value(&args, "--mode")?;
            write_file(&value(&args, "--output")?, vec![measure_mode(&mode)?])
        }
        Some("compare") => {
            let modes = value(&args, "--modes")?.split(',').map(str::to_owned).collect::<Vec<_>>();
            let mut results = Vec::new(); for mode in modes { results.push(measure_mode(&mode)?); }
            let output = value(&args, "--output")?; write_file(&output, results)?;
            if let Ok(report) = value(&args, "--report") {
                let reporter = sibling_reporter()?;
                let status = Command::new(reporter).args(["--input", &output, "--output", &report]).status().map_err(err)?;
                if !status.success() { return Err("performance gates failed; report contains evidence".into()); }
            }
            Ok(())
        }
        _ => Err("usage: jit-bench run --mode MODE --output FILE | compare --modes ... --output FILE --report FILE".into()),
    }
}

fn measure_mode(mode: &str) -> Result<ModeResult, String> {
    if !matches!(mode, "interpreter" | "tier1" | "tier2" | "automatic") {
        return Err(format!("unsupported mode {mode}"));
    }
    let scripts = [
        ("numeric", "compute", true, "numeric.js"),
        ("collections", "compute", false, "collections.js"),
        ("strings-json", "compute", false, "strings-json.js"),
        ("calls-closures", "compute", false, "calls-closures.js"),
        ("adversarial", "adversarial", false, "adversarial.js"),
    ];
    let executable = env::current_exe().map_err(err)?;
    let mut workloads = Vec::new();
    for (name, group, designated, file) in scripts {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(file);
        for _ in 0..WARMUPS {
            let _ = child(&executable, mode, &path)?;
        }
        let mut samples = Vec::new();
        let mut proof = None;
        for _ in 0..SAMPLES {
            let sample = child(&executable, mode, &path)?;
            samples.push(sample.elapsed_ns);
            proof = Some(sample);
        }
        let mut throughput = Vec::new();
        for _ in 0..WINDOWS {
            let start = Instant::now();
            let mut operations = 0;
            while start.elapsed() < WINDOW {
                let _ = child(&executable, mode, &path)?;
                operations += 1;
            }
            throughput.push(operations);
        }
        let proof = proof.ok_or("no sample")?;
        let (median, mad, p95, p99, ci) = model::summarize(samples.clone());
        workloads.push(WorkloadResult {
            name: name.into(),
            group: group.into(),
            designated_kernel: designated,
            raw_latency_ns: samples,
            raw_throughput_ops: throughput,
            median_ns: median,
            mad_ns: mad,
            p95_ns: p95,
            p99_ns: p99,
            ci95_ns: ci,
            checksum: proof.checksum,
            native_entries: proof.native_entries,
            native_exits: proof.native_exits,
            fallback_count: proof.fallbacks,
            retry_count: proof.retries,
            tier1_entries: proof.native_entries.saturating_sub(proof.tier2_entries),
            tier2_entries: proof.tier2_entries,
            osr_entries: proof.osr_entries,
            compile_ns: 0,
            install_ns: 0,
            break_even_executions: None,
            peak_rss_bytes: peak_rss(),
            code_bytes: proof.code_bytes,
            metadata_bytes: proof.metadata_bytes,
            peak_compiler_bytes: proof.active_ir_bytes,
        });
    }
    Ok(ModeResult {
        mode: mode.into(),
        workloads,
    })
}

fn worker(mode: &str, script: &str) -> Result<(), String> {
    let source = fs::read_to_string(script).map_err(err)?;
    let runtime = Runtime::new().map_err(err)?;
    let jit = if mode == "interpreter" {
        None
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
        Some(Jit::attach(&runtime, builder.build().map_err(err)?).map_err(err)?)
    };
    let context = Context::full(&runtime).map_err(err)?;
    let start = Instant::now();
    context
        .with(|ctx| -> rquickjs::Result<()> {
            ctx.eval::<(), _>(source.as_bytes())?;
            Ok(())
        })
        .map_err(err)?;
    let mut checksum = String::new();
    for _ in 0..8 {
        checksum = context
            .with(|ctx| -> rquickjs::Result<String> {
                let mut result = String::new();
                for _ in 0..10 {
                    result = ctx.eval("String(workload(2000, 0))")?;
                }
                Ok(result)
            })
            .map_err(err)?;
        if let Some(jit) = &jit {
            jit.poll();
        }
    }
    let elapsed_ns = start.elapsed().as_nanos().try_into().unwrap_or(u64::MAX);
    let metrics = jit.as_ref().map(Jit::metrics).unwrap_or_default();
    let result = WorkerResult {
        elapsed_ns,
        checksum,
        native_entries: metrics.native_entries,
        native_exits: metrics.native_exits,
        fallbacks: metrics.native_fallbacks,
        retries: metrics.native_retries,
        tier2_entries: metrics.tier2_entries,
        osr_entries: metrics.osr_entries,
        code_bytes: metrics.code_bytes as u64,
        metadata_bytes: metrics.metadata_bytes as u64,
        active_ir_bytes: metrics.active_ir_bytes as u64,
    };
    println!("{}", serde_json::to_string(&result).map_err(err)?);
    Ok(())
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
        },
        modes,
        exclusions: vec![Exclusion {
            suite: "SunSpider/JetStream".into(),
            test: "external corpora".into(),
            reason: "not imported; suites.lock records explicit status".into(),
        }],
    };
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).map_err(err)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&file).map_err(err)?).map_err(err)
}

fn provenance() -> Result<Provenance, String> {
    Ok(Provenance {
        source_revision: command("git", &["rev-parse", "HEAD"]),
        quickjs_revision: command("git", &["-C", "sys/quickjs", "rev-parse", "HEAD"]),
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
    })
}
fn peak_rss() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024
}
fn command(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
        .unwrap_or_else(|| "unknown".into())
}
fn value(args: &[String], flag: &str) -> Result<String, String> {
    args.iter()
        .position(|x| x == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .ok_or_else(|| format!("missing {flag}"))
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
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
