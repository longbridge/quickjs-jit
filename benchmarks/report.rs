mod model;
use model::{BenchmarkFile, ModeResult, SampleEvidence, WorkloadResult};
use std::{env, fs, path::Path, process::Command};

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
    if names != ["automatic", "interpreter", "tier1", "tier2"]
        && names != ["automatic", "bun", "interpreter", "tier1", "tier2"]
    {
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
    if data.provenance.source_dirty {
        return Err("benchmark evidence must come from a clean source tree".into());
    }
    if data.provenance.source_revision != command("git", &["rev-parse", "HEAD"])
        || data.provenance.quickjs_revision
            != command("git", &["-C", "../sys/quickjs", "rev-parse", "HEAD"])
    {
        return Err("source revision does not match the current checkout".into());
    }
    if data.provenance.schema_sha256 != current_sha256("schema/jit-benchmark-v1.json") {
        return Err("schema hash does not match the current file".into());
    }
    if data.provenance.suites_lock_sha256 != current_sha256("suites.lock") {
        return Err("suites lock hash does not match the current file".into());
    }
    let reference = data
        .modes
        .iter()
        .find(|mode| mode.mode == "interpreter")
        .ok_or("missing interpreter mode")?;
    let reference_shape = workload_shape(reference);
    let mut global_opcode = None;
    let mut global_abi = None;
    for mode in &data.modes {
        if workload_shape(mode) != reference_shape {
            return Err(format!(
                "{} workload set/suite/group/designated metadata differs",
                mode.mode
            ));
        }
        let mut mode_config = None;
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
                if s.elapsed_ns != w.raw_latency_ns[i] {
                    return Err(format!(
                        "{} / {} raw latency differs from sample",
                        mode.mode, w.name
                    ));
                }
                if mode.mode != "bun" {
                    match global_opcode {
                        Some(v) if v != s.opcode_fingerprint => {
                            return Err("opcode fingerprint drift".into())
                        }
                        None => global_opcode = Some(s.opcode_fingerprint),
                        _ => {}
                    }
                    match global_abi {
                        Some(v) if v != s.abi_fingerprint => {
                            return Err("ABI fingerprint drift".into())
                        }
                        None => global_abi = Some(s.abi_fingerprint),
                        _ => {}
                    }
                    match mode_config {
                        Some(v) if v != s.config_fingerprint => {
                            return Err(format!("{} config fingerprint drift", mode.mode))
                        }
                        None => mode_config = Some(s.config_fingerprint),
                        _ => {}
                    }
                }
            }
            let expected_checksum = reference
                .workloads
                .iter()
                .find(|x| x.name == w.name)
                .and_then(|x| x.samples.first())
                .map(|x| x.checksum.as_str())
                .ok_or("missing reference checksum")?;
            if !w
                .samples
                .iter()
                .all(|sample| sample.checksum == expected_checksum)
            {
                return Err(format!("{} / {} checksum mismatch", mode.mode, w.name));
            }
            let (median, mad, p95, p99, ci) = model::summarize(w.raw_latency_ns.clone());
            if (w.median_ns, w.mad_ns, w.p95_ns, w.p99_ns, w.ci95_ns) != (median, mad, p95, p99, ci)
                || w.compile_ns != median_field(&w.samples, |s| s.phases.compile_ns)
                || w.install_ns != median_field(&w.samples, |s| s.phases.install_ns)
            {
                return Err(format!(
                    "{} / {} recomputed summary mismatch",
                    mode.mode, w.name
                ));
            }
        }
    }
    Ok(())
}

fn workload_shape(mode: &ModeResult) -> Vec<(&str, &str, &str, bool)> {
    mode.workloads
        .iter()
        .map(|w| {
            (
                w.name.as_str(),
                w.suite.as_str(),
                w.group.as_str(),
                w.designated_kernel,
            )
        })
        .collect()
}

fn median_field(samples: &[SampleEvidence], field: impl Fn(&SampleEvidence) -> u64) -> u64 {
    let mut values = samples.iter().map(field).collect::<Vec<_>>();
    values.sort_unstable();
    model::quantile(&values, 0.5)
}

fn command(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn current_sha256(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    command("sha256sum", &[path.to_string_lossy().as_ref()])
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn render(data: &BenchmarkFile) -> (String, bool) {
    let interp = mode(data, "interpreter");
    let t1 = mode(data, "tier1");
    let t2 = mode(data, "tier2");
    let auto = mode(data, "automatic");
    let bun = mode(data, "bun");
    let mut out=format!("# JIT performance report\n\nStatus: generated from tracked raw `jit-benchmark-v1` evidence. Source `{}` (dirty: {}), QuickJS `{}`; target `{}`; CPU `{}`; power `{}`. Bun: `{}` at `{}` (SHA-256 `{}`).\n\nCommand: `{}`. Schema SHA-256 `{}`; suites lock SHA-256 `{}`.\n\nSampling: {} discarded warmup processes, {} interleaved paired fresh processes, {} interleaved one-second throughput windows, {} joint paired bootstrap resamples.\n\n## Workloads\n\nA JIT ratio is reported only when that mode actually entered native code; fallback-only timing is shown as `N/A (no native entry)`. Bun remains an external engine comparison.\n\n| workload (suite) | interpreter median ns | Tier1 | Tier2 | automatic | Bun | T1/T2 entries | fallback/retry | checksum |\n|---|---:|---:|---:|---:|---:|---:|---:|---|\n",data.provenance.source_revision,data.provenance.source_dirty,data.provenance.quickjs_revision,data.provenance.target,data.provenance.cpu.replace('\n'," "),data.provenance.power_mode,data.provenance.bun_version.as_deref().unwrap_or("N/A"),data.provenance.bun_path.as_deref().unwrap_or("N/A"),data.provenance.bun_sha256.as_deref().unwrap_or("N/A"),data.provenance.command.join(" "),data.provenance.schema_sha256,data.provenance.suites_lock_sha256,data.policy.latency_warmups,data.policy.latency_processes,data.policy.throughput_windows,data.policy.bootstrap_resamples);
    let mut checksums = true;
    let mut strict_native = true;
    let mut automatic_policy = false;
    if let Some(base) = interp {
        for w in &base.workloads {
            let a = matching(auto, w);
            let one = matching(t1, w);
            let two = matching(t2, w);
            let external = matching(bun, w);
            checksums &= one.is_some_and(|x| all_checksums(x, checksum(w)))
                && two.is_some_and(|x| all_checksums(x, checksum(w)))
                && a.is_some_and(|x| all_checksums(x, checksum(w)));
            if bun.is_some() {
                checksums &= external.is_some_and(|x| all_checksums(x, checksum(w)));
            }
            if w.designated_kernel {
                strict_native &= one.is_some_and(|x| {
                    x.samples
                        .iter()
                        .all(|s| s.tier1_entries.unwrap_or(0) > 0 && s.tier2_entries == Some(0))
                }) && two
                    .is_some_and(|x| x.samples.iter().all(|s| s.tier2_entries.unwrap_or(0) > 0));
            }
            automatic_policy |= a.is_some_and(|x| {
                x.samples
                    .iter()
                    .all(|s| s.profitability_evaluations.unwrap_or(0) > 0)
                    && x.samples
                        .iter()
                        .any(|s| s.profitability_approved.unwrap_or(0) > 0)
            });
            let proof = two.or(a);
            let entries = proof
                .map(|x| {
                    (
                        sum(x, |s| s.tier1_entries.unwrap_or(0)),
                        sum(x, |s| s.tier2_entries.unwrap_or(0)),
                    )
                })
                .unwrap_or_default();
            let exits = proof
                .map(|x| {
                    (
                        sum(x, |s| s.fallback_count.unwrap_or(0)),
                        sum(x, |s| s.retry_count.unwrap_or(0)),
                    )
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "| {} ({}) | {} | {} | {} | {} | {} | {}/{} | {}/{} | `{}` |\n",
                w.name,
                w.suite,
                w.median_ns,
                fmt_jit_speedup(w, one, |s| s.tier1_entries.unwrap_or(0)),
                fmt_jit_speedup(w, two, |s| s.tier2_entries.unwrap_or(0)),
                fmt_jit_speedup(w, a, |s| s.native_entries.unwrap_or(0)),
                fmt(speedup(w, external)),
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
    out.push_str(&format!(
        "\nStripped binary evidence: non-JIT {} bytes; JIT {} bytes; delta {:+} bytes.\n",
        data.provenance.stripped_no_jit_bytes,
        data.provenance.stripped_jit_bytes,
        data.provenance.stripped_jit_delta_bytes
    ));
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
fn fmt_jit_speedup(
    base: &WorkloadResult,
    candidate: Option<&WorkloadResult>,
    entries: impl Fn(&SampleEvidence) -> u64,
) -> String {
    match candidate {
        Some(candidate) if candidate.samples.iter().any(|sample| entries(sample) > 0) => {
            fmt(speedup(base, Some(candidate)))
        }
        Some(_) => "N/A (no native entry)".into(),
        None => "—".into(),
    }
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
            native_entries: Some(1),
            native_exits: Some(1),
            fallback_count: Some(0),
            retry_count: Some(0),
            tier1_entries: Some(1),
            tier2_entries: Some(0),
            deopt_count: Some(0),
            osr_attempts: Some(0),
            profitability_evaluations: Some(1),
            profitability_approved: Some(1),
            profitability_rejected: Some(0),
            benefit_recordings: Some(1),
            measured_benefit_ns: Some(1),
            opcode_fingerprint: Some(1),
            abi_fingerprint: Some(1),
            config_fingerprint: Some(1),
            peak_rss_bytes: Some(1),
            code_bytes: Some(1),
            metadata_bytes: Some(1),
            peak_compiler_bytes: Some(1),
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
            compile_ns: 0,
            install_ns: 0,
            break_even_executions: Some(1),
        }
    }
    fn valid_file() -> BenchmarkFile {
        let workloads = vec![work("a", 100), work("b", 200)];
        BenchmarkFile {
            schema: "jit-benchmark-v1".into(),
            provenance: model::Provenance {
                source_revision: command("git", &["rev-parse", "HEAD"]),
                quickjs_revision: command("git", &["-C", "../sys/quickjs", "rev-parse", "HEAD"]),
                source_dirty: false,
                command: vec!["jit-bench".into()],
                target: "x86_64".into(),
                target_triple: "x86_64-unknown-linux-gnu".into(),
                os: "linux".into(),
                kernel: "test".into(),
                cpu: "test".into(),
                power_mode: "test".into(),
                rustc: "test".into(),
                llvm: "test".into(),
                executable_bytes: 1,
                stripped_jit_bytes: 2,
                stripped_no_jit_bytes: 1,
                stripped_jit_delta_bytes: 1,
                schema_sha256: current_sha256("schema/jit-benchmark-v1.json"),
                suites_lock_sha256: current_sha256("suites.lock"),
                bun_version: None,
                bun_path: None,
                bun_sha256: None,
            },
            policy: model::SamplingPolicy {
                latency_warmups: 5,
                latency_processes: 30,
                throughput_windows: 10,
                throughput_window_ns: 1_000_000_000,
                bootstrap_resamples: 10_000,
                pairing: "paired".into(),
            },
            modes: ["interpreter", "tier1", "tier2", "automatic"]
                .into_iter()
                .map(|mode| ModeResult {
                    mode: mode.into(),
                    workloads: workloads.clone(),
                })
                .collect(),
            exclusions: vec![],
        }
    }
    #[test]
    fn validator_rejects_dirty_or_stale_provenance() {
        let mut data = valid_file();
        assert!(validate(&data).is_ok());
        data.provenance.source_dirty = true;
        assert!(validate(&data).unwrap_err().contains("clean"));
        data.provenance.source_dirty = false;
        data.provenance.schema_sha256 = "0".repeat(64);
        assert!(validate(&data).unwrap_err().contains("schema"));
        data.provenance.schema_sha256 = current_sha256("schema/jit-benchmark-v1.json");
        data.provenance.suites_lock_sha256 = "0".repeat(64);
        assert!(validate(&data).unwrap_err().contains("suites"));
    }
    #[test]
    fn validator_rejects_cross_mode_shape_and_identity_drift() {
        let mut data = valid_file();
        data.modes[1].workloads[0].suite = "wrong".into();
        assert!(validate(&data).unwrap_err().contains("workload set"));
        let mut data = valid_file();
        data.modes[2].workloads[0].samples[3].checksum = "wrong".into();
        assert!(validate(&data).unwrap_err().contains("checksum"));
        let mut data = valid_file();
        data.modes[3].workloads[1].samples[4].opcode_fingerprint = Some(2);
        assert!(validate(&data).unwrap_err().contains("fingerprint"));
    }
    #[test]
    fn validator_rejects_raw_or_recomputed_summary_drift() {
        let mut data = valid_file();
        data.modes[0].workloads[0].raw_latency_ns[2] += 1;
        assert!(validate(&data).unwrap_err().contains("raw latency"));
        let mut data = valid_file();
        data.modes[0].workloads[0].median_ns += 1;
        assert!(validate(&data).unwrap_err().contains("summary"));
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

    #[test]
    fn fallback_only_timing_is_not_rendered_as_jit_speedup() {
        let base = work("matrix", 200);
        let mut fallback = work("matrix", 100);
        fallback.designated_kernel = false;
        for sample in &mut fallback.samples {
            sample.native_entries = Some(0);
            sample.tier1_entries = Some(0);
            sample.tier2_entries = Some(0);
        }
        assert_eq!(
            fmt_jit_speedup(&base, Some(&fallback), |s| s.native_entries.unwrap_or(0)),
            "N/A (no native entry)"
        );
        fallback.samples[0].native_entries = Some(1);
        assert_eq!(
            fmt_jit_speedup(&base, Some(&fallback), |s| s.native_entries.unwrap_or(0)),
            "2.00×"
        );
    }
}
