mod model;
use model::{BenchmarkFile, ModeResult, SampleEvidence, WorkloadResult};
use std::{env, fs};

fn main() {
    match real_main() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    }
}
fn real_main() -> Result<bool, String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let input = value(&args, "--input")?;
    let output = value(&args, "--output")?;
    let data: BenchmarkFile =
        serde_json::from_slice(&fs::read(&input).map_err(err)?).map_err(err)?;
    validate(&data)?;
    let (markdown, pass) = render(&data);
    if let Some(parent) = std::path::Path::new(&output).parent() {
        fs::create_dir_all(parent).map_err(err)?
    }
    fs::write(output, markdown).map_err(err)?;
    Ok(pass)
}

fn validate(data: &BenchmarkFile) -> Result<(), String> {
    if data.schema != "jit-benchmark-v1" {
        return Err("unsupported schema".into());
    }
    if data.policy.latency_warmups != 5
        || data.policy.latency_processes != 30
        || data.policy.throughput_windows != 10
        || data.policy.throughput_window_ns != 1_000_000_000
        || data.policy.bootstrap_resamples < 10_000
    {
        return Err("sampling policy below required minimum".into());
    }
    let mut names = data
        .modes
        .iter()
        .map(|m| m.mode.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    if names != ["automatic", "interpreter", "tier1", "tier2"] {
        return Err(
            "modes must contain each of automatic/interpreter/tier1/tier2 exactly once".into(),
        );
    }
    if data.provenance.source_revision.len() != 40
        || data.provenance.quickjs_revision.len() != 40
        || data.provenance.target_triple.split('-').count() < 3
    {
        return Err("incomplete source hashes or target triple".into());
    }
    for mode in &data.modes {
        for w in &mode.workloads {
            if w.samples.len() != data.policy.latency_processes as usize {
                return Err(format!("{} / {} sample count mismatch", mode.mode, w.name));
            }
            if w.raw_latency_ns.len() != 30 || w.raw_throughput_ops.len() != 10 {
                return Err(format!(
                    "{} / {} raw sample count mismatch",
                    mode.mode, w.name
                ));
            }
            for (i, s) in w.samples.iter().enumerate() {
                if s.pair_index as usize != i {
                    return Err(format!("{} / {} invalid pair index", mode.mode, w.name));
                }
            }
        }
    }
    Ok(())
}

fn render(data: &BenchmarkFile) -> (String, bool) {
    let interp = mode(data, "interpreter");
    let t1 = mode(data, "tier1");
    let t2 = mode(data, "tier2");
    let auto = mode(data, "automatic");
    let mut out=format!("# JIT performance report\n\nStatus: generated from tracked raw `jit-benchmark-v1` evidence. Source `{}` (dirty: {}), QuickJS `{}`; target `{}`; CPU `{}`; power `{}`.\n\nCommand: `{}`. Schema SHA-256 `{}`; suites lock SHA-256 `{}`.\n\nSampling: {} discarded warmup processes, {} interleaved paired fresh processes, {} interleaved one-second throughput windows, {} joint paired bootstrap resamples.\n\n## Workloads\n\n| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | T1/T2 entries | fallback/retry | checksum |\n|---|---:|---:|---:|---:|---:|---:|---|\n",data.provenance.source_revision,data.provenance.source_dirty,data.provenance.quickjs_revision,data.provenance.target,data.provenance.cpu.replace('\n'," "),data.provenance.power_mode,data.provenance.command.join(" "),data.provenance.schema_sha256,data.provenance.suites_lock_sha256,data.policy.latency_warmups,data.policy.latency_processes,data.policy.throughput_windows,data.policy.bootstrap_resamples);
    out.push_str(&format!(
        "\nStripped binary evidence: non-JIT {} bytes; JIT {} bytes; delta {:+} bytes.\n",
        data.provenance.stripped_no_jit_bytes,
        data.provenance.stripped_jit_bytes,
        data.provenance.stripped_jit_delta_bytes
    ));
    let mut checksums = true;
    let mut strict_native = true;
    let mut automatic_policy = false;
    if let Some(base) = interp {
        for w in &base.workloads {
            let a = matching(auto, w);
            let one = matching(t1, w);
            let two = matching(t2, w);
            checksums &= one.is_some_and(|x| all_checksums(x, checksum(w)))
                && two.is_some_and(|x| all_checksums(x, checksum(w)))
                && a.is_some_and(|x| all_checksums(x, checksum(w)));
            strict_native &= one.is_some_and(|x| {
                x.samples
                    .iter()
                    .all(|s| s.tier1_entries > 0 && s.tier2_entries == 0)
            }) && two
                .is_some_and(|x| x.samples.iter().all(|s| s.tier2_entries > 0));
            automatic_policy |= a.is_some_and(|x| {
                x.samples.iter().all(|s| s.profitability_evaluations > 0)
                    && x.samples.iter().any(|s| s.profitability_approved > 0)
            });
            let proof = two.or(a);
            let entries = proof
                .map(|x| (sum(x, |s| s.tier1_entries), sum(x, |s| s.tier2_entries)))
                .unwrap_or_default();
            let exits = proof
                .map(|x| (sum(x, |s| s.fallback_count), sum(x, |s| s.retry_count)))
                .unwrap_or_default();
            out.push_str(&format!(
                "| {} ({}) | {} | {} | {} | {} | {}/{} | {}/{} | `{}` |\n",
                w.name,
                w.suite,
                w.median_ns,
                fmt(speedup(w, one)),
                fmt(speedup(w, two)),
                fmt(speedup(w, a)),
                entries.0,
                entries.1,
                exits.0,
                exits.1,
                checksum(w)
            ));
        }
    } else {
        checksums = false;
        strict_native = false;
        automatic_policy = false
    }
    let compute_ci = joint_geomean_ci(interp, t2, "compute");
    let compute_pass = compute_ci.is_some_and(|ci| ci[0] >= 5.0);
    let kernels = interp
        .into_iter()
        .flat_map(|m| &m.workloads)
        .filter(|w| w.designated_kernel)
        .map(|w| {
            (
                w.name.clone(),
                matching(t2, w).and_then(|x| paired_ratio_ci(w, x, |s| s.phases.steady_state_ns)),
            )
        })
        .collect::<Vec<_>>();
    let kernel_pass = !kernels.is_empty()
        && kernels
            .iter()
            .any(|(_, ci)| ci.is_some_and(|x| x[0] >= 10.0));
    let startup = joint_phase_regression_ci(interp, auto, |s| {
        s.phases
            .runtime_create_ns
            .saturating_add(s.phases.jit_attach_ns)
            .saturating_add(s.phases.context_create_ns)
            .saturating_add(s.phases.first_eval_ns)
    });
    let reload = joint_phase_regression_ci(interp, auto, |s| s.phases.definition_eval_ns);
    let p99 = joint_p99_regression_ci(interp, auto);
    let regression_pass = [startup, reload, p99]
        .into_iter()
        .all(|ci| ci.is_some_and(|x| x[1] <= 1.05));
    out.push_str("\n## Acceptance gates\n\n");
    let mut all = true;
    all &= gate(
        &mut out,
        "Compute paired geometric-mean lower CI ≥5×",
        compute_pass,
        compute_ci
            .map(|x| format!("{:.2}×..{:.2}×", x[0], x[1]))
            .unwrap_or_else(|| "INCONCLUSIVE".into()),
    );
    all &= gate(
        &mut out,
        "At least one designated kernel lower CI ≥10×",
        kernel_pass,
        format!("{:?}", kernels),
    );
    all &= gate(
        &mut out,
        "Every strict sample has required native tier",
        strict_native,
        if strict_native {
            "all samples"
        } else {
            "FAIL: missing per-sample entry"
        }
        .into(),
    );
    all &= gate(
        &mut out,
        "Automatic uses production profitability policy",
        automatic_policy,
        if automatic_policy {
            "all samples evaluated"
        } else {
            "FAIL: missing decision"
        }
        .into(),
    );
    all &= gate(
        &mut out,
        "Checksums identical in every sample",
        checksums,
        if checksums {
            "all samples"
        } else {
            "FAIL: mismatch"
        }
        .into(),
    );
    all &= gate(
        &mut out,
        "startup/hot-reload/P99 upper regression CI ≤5%",
        regression_pass,
        format!("startup={startup:?}, reload={reload:?}, p99={p99:?}"),
    );
    all &= gate(
        &mut out,
        "gpui-shell steady state ≥2×",
        false,
        "INCONCLUSIVE: Task 15 worktree evidence not supplied".into(),
    );
    out.push_str("\n## Phase, break-even, and memory evidence\n\nEvery raw sample retains cold runtime creation, JIT attach, context creation, definition/first eval, threshold crossing, measured compile/install, OSR, and steady-state timing; worker VmHWM RSS; code/metadata/compiler high-water memory; native entry/exit, OSR attempts, retry/fallback, profitability, benefit, and configuration/ABI/opcode fingerprints. Helper-exit attribution is not exposed by current runtime metrics and is intentionally absent. Break-even is compile+install cost divided by paired end-to-end savings and is null when no saving was observed.\n\n## Exclusions\n\n");
    for e in &data.exclusions {
        out.push_str(&format!("- {} / {}: {}\n", e.suite, e.test, e.reason));
    }
    out.push_str("\nQuickJS `int_arith` is adapted under MIT from the pinned local `sys/quickjs/tests/microbench.js`. SunSpider and JetStream are not represented by placeholders. Missing or failed evidence remains FAIL/INCONCLUSIVE.\n");
    (out, all)
}

fn joint_geomean_ci(
    base: Option<&ModeResult>,
    candidate: Option<&ModeResult>,
    group: &str,
) -> Option<[f64; 2]> {
    joint_bootstrap(base?, candidate?, |b, c, index| {
        let pairs = b
            .workloads
            .iter()
            .filter(|w| w.group == group)
            .filter_map(|w| matching(Some(c), w).map(|x| (w, x)))
            .collect::<Vec<_>>();
        if pairs.is_empty() {
            return None;
        }
        let sum = pairs
            .iter()
            .map(|(x, y)| {
                (sample(x, index).elapsed_ns as f64 / sample(y, index).elapsed_ns as f64).ln()
            })
            .sum::<f64>();
        Some((sum / pairs.len() as f64).exp())
    })
}
fn joint_phase_regression_ci(
    base: Option<&ModeResult>,
    candidate: Option<&ModeResult>,
    field: impl Fn(&SampleEvidence) -> u64 + Copy,
) -> Option<[f64; 2]> {
    joint_bootstrap(base?, candidate?, |b, c, index| {
        let pairs = b
            .workloads
            .iter()
            .filter_map(|w| matching(Some(c), w).map(|x| (w, x)))
            .collect::<Vec<_>>();
        let mut logs = Vec::new();
        for (x, y) in pairs {
            let bv = field(sample(x, index));
            let cv = field(sample(y, index));
            if bv > 0 && cv > 0 {
                logs.push((cv as f64 / bv as f64).ln())
            }
        }
        (!logs.is_empty()).then(|| (logs.iter().sum::<f64>() / logs.len() as f64).exp())
    })
}
fn joint_p99_regression_ci(
    base: Option<&ModeResult>,
    candidate: Option<&ModeResult>,
) -> Option<[f64; 2]> {
    let b = base?;
    let c = candidate?;
    let mut state = 0x510e527fade682d1u64;
    let n = b.workloads.first()?.samples.len();
    let mut reps = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let indices = (0..n)
            .map(|_| {
                state = model::xorshift(state);
                state as usize % n
            })
            .collect::<Vec<_>>();
        let mut ratios = Vec::new();
        for bw in &b.workloads {
            let cw = matching(Some(c), bw)?;
            let mut bv = indices
                .iter()
                .map(|i| sample(bw, *i).elapsed_ns)
                .collect::<Vec<_>>();
            let mut cv = indices
                .iter()
                .map(|i| sample(cw, *i).elapsed_ns)
                .collect::<Vec<_>>();
            bv.sort_unstable();
            cv.sort_unstable();
            let bp = model::quantile(&bv, 0.99);
            let cp = model::quantile(&cv, 0.99);
            if bp > 0 {
                ratios.push((cp as f64 / bp as f64).ln())
            }
        }
        if !ratios.is_empty() {
            reps.push((ratios.iter().sum::<f64>() / ratios.len() as f64).exp())
        }
    }
    interval(reps)
}
fn joint_bootstrap(
    base: &ModeResult,
    candidate: &ModeResult,
    stat: impl Fn(&ModeResult, &ModeResult, usize) -> Option<f64>,
) -> Option<[f64; 2]> {
    let n = base.workloads.first()?.samples.len();
    if n == 0 {
        return None;
    }
    let mut state = 0x1f83d9abfb41bd6bu64;
    let mut reps = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let mut logs = Vec::with_capacity(n);
        for _ in 0..n {
            state = model::xorshift(state);
            if let Some(v) = stat(base, candidate, state as usize % n) {
                logs.push(v.ln())
            }
        }
        if !logs.is_empty() {
            reps.push((logs.iter().sum::<f64>() / logs.len() as f64).exp())
        }
    }
    interval(reps)
}
fn paired_ratio_ci(
    base: &WorkloadResult,
    candidate: &WorkloadResult,
    field: impl Fn(&SampleEvidence) -> u64,
) -> Option<[f64; 2]> {
    let ratios = base
        .samples
        .iter()
        .zip(&candidate.samples)
        .filter_map(|(b, c)| {
            let (b, c) = (field(b), field(c));
            (b > 0 && c > 0).then(|| b as f64 / c as f64)
        })
        .collect::<Vec<_>>();
    if ratios.is_empty() {
        return None;
    }
    let mut state = 0xa54ff53a5f1d36f1u64;
    let mut reps = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let mut log = 0.0;
        for _ in 0..ratios.len() {
            state = model::xorshift(state);
            log += ratios[state as usize % ratios.len()].ln()
        }
        reps.push((log / ratios.len() as f64).exp())
    }
    interval(reps)
}
fn interval(mut v: Vec<f64>) -> Option<[f64; 2]> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some([
        model::quantile_f64(&v, 0.025),
        model::quantile_f64(&v, 0.975),
    ])
}
fn sample(w: &WorkloadResult, index: usize) -> &SampleEvidence {
    &w.samples[index % w.samples.len()]
}
fn checksum(w: &WorkloadResult) -> &str {
    w.samples.first().map(|s| s.checksum.as_str()).unwrap_or("")
}
fn all_checksums(w: &WorkloadResult, expected: &str) -> bool {
    w.samples.iter().all(|s| s.checksum == expected)
}
fn sum(w: &WorkloadResult, f: impl Fn(&SampleEvidence) -> u64) -> u64 {
    w.samples.iter().map(f).sum()
}
fn speedup(base: &WorkloadResult, candidate: Option<&WorkloadResult>) -> Option<f64> {
    candidate.map(|x| base.median_ns as f64 / x.median_ns as f64)
}
fn matching<'a>(m: Option<&'a ModeResult>, w: &WorkloadResult) -> Option<&'a WorkloadResult> {
    m?.workloads.iter().find(|x| x.name == w.name)
}
fn mode<'a>(d: &'a BenchmarkFile, name: &str) -> Option<&'a ModeResult> {
    d.modes.iter().find(|m| m.mode == name)
}
fn fmt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}×")).unwrap_or_else(|| "—".into())
}
fn gate(out: &mut String, name: &str, pass: bool, evidence: String) -> bool {
    out.push_str(&format!(
        "- {} — **{}**: {}\n",
        if pass { "PASS" } else { "FAIL/INCONCLUSIVE" },
        name,
        evidence
    ));
    pass
}
fn value(args: &[String], flag: &str) -> Result<String, String> {
    args.iter()
        .position(|x| x == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .ok_or_else(|| format!("missing {flag}"))
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence(pair: u32, b: u64, c: &str) -> SampleEvidence {
        SampleEvidence {
            pair_index: pair,
            elapsed_ns: b,
            checksum: c.into(),
            native_entries: 1,
            native_exits: 1,
            fallback_count: 0,
            retry_count: 0,
            tier1_entries: 1,
            tier2_entries: 0,
            osr_attempts: 0,
            profitability_evaluations: 1,
            profitability_approved: 1,
            profitability_rejected: 0,
            benefit_recordings: 1,
            measured_benefit_ns: 1,
            opcode_fingerprint: 1,
            abi_fingerprint: 1,
            config_fingerprint: 1,
            peak_rss_bytes: 1,
            code_bytes: 1,
            metadata_bytes: 1,
            peak_compiler_bytes: 1,
            phases: model::PhaseTiming {
                steady_state_ns: b,
                ..Default::default()
            },
        }
    }
    fn work(name: &str, time: u64) -> WorkloadResult {
        let samples = (0..30).map(|i| evidence(i, time, "x")).collect();
        WorkloadResult {
            name: name.into(),
            suite: "x".into(),
            group: "compute".into(),
            designated_kernel: true,
            samples,
            raw_latency_ns: vec![time; 30],
            raw_throughput_ops: vec![1; 10],
            median_ns: time,
            mad_ns: 0,
            p95_ns: time,
            p99_ns: time,
            ci95_ns: [time, time],
            compile_ns: 1,
            install_ns: 1,
            break_even_executions: Some(1),
        }
    }
    #[test]
    fn joint_ci_preserves_consistent_cross_workload_ratio() {
        let b = ModeResult {
            mode: "interpreter".into(),
            workloads: vec![work("a", 200), work("b", 400)],
        };
        let c = ModeResult {
            mode: "tier2".into(),
            workloads: vec![work("a", 100), work("b", 200)],
        };
        let ci = joint_geomean_ci(Some(&b), Some(&c), "compute").unwrap();
        assert!(ci[0] > 1.99 && ci[1] < 2.01)
    }
}
