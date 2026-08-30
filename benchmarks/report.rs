mod model;

use model::{BenchmarkFile, ModeResult, WorkloadResult};
use std::{env, fs};

fn main() {
    match real_main() {
        Ok(pass) if pass => {}
        Ok(_) => std::process::exit(1),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

fn real_main() -> Result<bool, String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let input = value(&args, "--input")?;
    let output = value(&args, "--output")?;
    let data: BenchmarkFile =
        serde_json::from_slice(&fs::read(&input).map_err(err)?).map_err(err)?;
    let (markdown, pass) = render(&data);
    if let Some(parent) = std::path::Path::new(&output).parent() {
        fs::create_dir_all(parent).map_err(err)?;
    }
    fs::write(output, markdown).map_err(err)?;
    Ok(pass)
}

fn render(data: &BenchmarkFile) -> (String, bool) {
    let interpreter = mode(data, "interpreter");
    let automatic = mode(data, "automatic");
    let tier1 = mode(data, "tier1");
    let tier2 = mode(data, "tier2");
    let mut report = format!("# JIT performance report\n\nStatus: generated from raw `jit-benchmark-v1` evidence. Source `{}`; target `{}`; CPU `{}`; power `{}`.\n\nSampling: {} discarded warmup processes, {} fresh measured processes, {} one-second throughput windows, 10,000 deterministic bootstrap resamples.\n\n## Workloads\n\n| workload | interpreter median ns | Tier1 speedup | Tier2 speedup | automatic speedup | native T1/T2 | fallback | checksum |\n|---|---:|---:|---:|---:|---:|---:|---|\n",
        data.provenance.source_revision, data.provenance.target, data.provenance.cpu.replace('\n', " "), data.provenance.power_mode,
        data.policy.latency_warmups, data.policy.latency_processes, data.policy.throughput_windows);
    let mut compute_logs = Vec::new();
    let mut kernel_pass = false;
    let mut evidence_complete = true;
    if let Some(base) = interpreter {
        for workload in &base.workloads {
            let t1 = matching(tier1, workload);
            let t2 = matching(tier2, workload);
            let auto = matching(automatic, workload);
            let s1 = speedup(workload, t1);
            let s2 = speedup(workload, t2);
            let sa = speedup(workload, auto);
            if workload.group == "compute" {
                if let Some(interval) = t2.and_then(|candidate| {
                    paired_log_ratio_ci(&workload.raw_latency_ns, &candidate.raw_latency_ns)
                }) {
                    compute_logs.push(interval[0].ln());
                }
            }
            if workload.designated_kernel
                && t2
                    .and_then(|candidate| {
                        paired_log_ratio_ci(&workload.raw_latency_ns, &candidate.raw_latency_ns)
                    })
                    .map(|interval| interval[0] >= 10.0)
                    .unwrap_or(false)
            {
                kernel_pass = true;
            }
            let proof = t2.or(auto);
            if proof.map(|x| x.checksum.as_str()) != Some(workload.checksum.as_str()) {
                evidence_complete = false;
            }
            report.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | `{}` |\n",
                workload.name,
                workload.median_ns,
                fmt(s1),
                fmt(s2),
                fmt(sa),
                proof
                    .map(|x| format!("{}/{}", x.tier1_entries, x.tier2_entries))
                    .unwrap_or_else(|| "—".into()),
                proof
                    .map(|x| x.fallback_count.to_string())
                    .unwrap_or_else(|| "—".into()),
                workload.checksum
            ));
        }
    } else {
        evidence_complete = false;
    }
    let geomean = if compute_logs.is_empty() {
        None
    } else {
        Some((compute_logs.iter().sum::<f64>() / compute_logs.len() as f64).exp())
    };
    let compute_pass = geomean.unwrap_or(0.0) >= 5.0;
    let native_proof = tier2
        .map(|m| m.workloads.iter().any(|w| w.tier2_entries > 0))
        .unwrap_or(false)
        && tier1
            .map(|m| m.workloads.iter().any(|w| w.tier1_entries > 0))
            .unwrap_or(false);
    report.push_str("\n## Acceptance gates\n\n");
    gate(
        &mut report,
        "Compute geometric mean ≥5×",
        compute_pass,
        geomean
            .map(|x| format!("{x:.2}×"))
            .unwrap_or_else(|| "INCONCLUSIVE: missing modes".into()),
    );
    gate(
        &mut report,
        "Designated hot kernel ≥10×",
        kernel_pass,
        if kernel_pass {
            "met".into()
        } else {
            "FAIL/INCONCLUSIVE".into()
        },
    );
    gate(
        &mut report,
        "Native Tier1 and Tier2 evidence",
        native_proof,
        if native_proof {
            "native entries observed".into()
        } else {
            "FAIL: native entry evidence absent".into()
        },
    );
    gate(
        &mut report,
        "Checksums identical",
        evidence_complete,
        if evidence_complete {
            "all compared checksums match".into()
        } else {
            "INCONCLUSIVE/mismatch".into()
        },
    );
    gate(
        &mut report,
        "gpui-shell steady-state ≥2×",
        false,
        "INCONCLUSIVE: Task 15 worktree evidence not yet supplied".into(),
    );
    gate(
        &mut report,
        "startup/hot-reload/P99 regression ≤5%",
        false,
        "INCONCLUSIVE: Task 15 worktree evidence not yet supplied".into(),
    );
    report.push_str("\n## Break-even and memory\n\nRaw JSON retains per-workload compile/install time, break-even execution, peak RSS, native code, metadata, compiler memory, entry/exit/retry/fallback counters, and every latency/throughput sample. A zero or null field means the runtime did not expose that measurement; it is not estimated. Binary size is provenance metadata and is kept separate from RSS.\n\n## Exclusions\n\n");
    for exclusion in &data.exclusions {
        report.push_str(&format!(
            "- {} / {}: {}\n",
            exclusion.suite, exclusion.test, exclusion.reason
        ));
    }
    report.push_str("\nThis report never converts missing evidence into a pass. Failed targets remain visible and must not be hidden by fallback or workload removal.\n");
    (
        report,
        compute_pass && kernel_pass && native_proof && evidence_complete && false,
    )
}

fn paired_log_ratio_ci(base: &[u64], candidate: &[u64]) -> Option<[f64; 2]> {
    let n = base.len().min(candidate.len());
    if n == 0 {
        return None;
    }
    let ratios = (0..n)
        .filter(|&i| base[i] > 0 && candidate[i] > 0)
        .map(|i| (base[i] as f64 / candidate[i] as f64).ln())
        .collect::<Vec<_>>();
    if ratios.is_empty() {
        return None;
    }
    let mut state = 0xbb67ae8584caa73bu64;
    let mut samples = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let mut sum = 0.0;
        for _ in 0..ratios.len() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            sum += ratios[(state as usize) % ratios.len()];
        }
        samples.push((sum / ratios.len() as f64).exp());
    }
    samples.sort_by(f64::total_cmp);
    Some([samples[249], samples[9749]])
}
fn speedup(base: &WorkloadResult, candidate: Option<&WorkloadResult>) -> Option<f64> {
    candidate.map(|x| base.median_ns as f64 / x.median_ns as f64)
}
fn matching<'a>(
    mode: Option<&'a ModeResult>,
    workload: &WorkloadResult,
) -> Option<&'a WorkloadResult> {
    mode?.workloads.iter().find(|x| x.name == workload.name)
}
fn mode<'a>(data: &'a BenchmarkFile, name: &str) -> Option<&'a ModeResult> {
    data.modes.iter().find(|x| x.mode == name)
}
fn fmt(value: Option<f64>) -> String {
    value
        .map(|x| format!("{x:.2}×"))
        .unwrap_or_else(|| "—".into())
}
fn gate(report: &mut String, name: &str, passed: bool, evidence: String) {
    report.push_str(&format!(
        "- {} — **{}**: {}\n",
        if passed { "PASS" } else { "FAIL/INCONCLUSIVE" },
        name,
        evidence
    ));
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
    fn paired_ci_detects_consistent_two_x_speedup() {
        let ci = paired_log_ratio_ci(&[200; 30], &[100; 30]).unwrap();
        assert!(ci[0] > 1.99 && ci[1] < 2.01);
    }
}
