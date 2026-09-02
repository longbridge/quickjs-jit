use serde::Deserialize;
use std::{env, fs};

const REQUIRED_SAMPLES: usize = 30;
const REQUIRED_BOOTSTRAPS: usize = 10_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema: String,
    provenance: Provenance,
    policy: Policy,
    workloads: Vec<Workload>,
    lifecycle: Lifecycle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    shell_revision: String,
    rquickjs_revision: String,
    shell_dirty: bool,
    rquickjs_dirty: bool,
    test_binary_sha256: String,
    integration_patch_sha256: String,
    cpu_affinity: String,
    target_triple: String,
    command: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    warmup_processes: usize,
    paired_processes: usize,
    bootstrap_resamples: usize,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workload {
    name: String,
    suitable_for_jit: bool,
    regression_guard: bool,
    interpreter: Vec<Sample>,
    automatic: Vec<Sample>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    pair_index: usize,
    steady_state_ns: u64,
    p99_script_render_ns: u64,
    checksum: String,
    snapshot_sha256: String,
    script_renders: u64,
    native_enabled: bool,
    native_entries: u64,
    fallback_count: u64,
    installed: u64,
    compile_failures: u64,
    unsupported_opcode_failures: u64,
    tier1_rejections: u64,
    resource_limit_failures: u64,
    cancelled_compilations: u64,
    compiler_panics: u64,
    invalid_artifacts: u64,
    install_failures: u64,
    native_exits: u64,
    osr_entries: u64,
    deopts: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lifecycle {
    interpreter: Vec<LifecycleSample>,
    automatic: Vec<LifecycleSample>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleSample {
    pair_index: usize,
    first_window_ns: u64,
    hot_reload_ns: u64,
    snapshot_sha256: String,
    script_renders: u64,
    reload_observations: Vec<serde_json::Value>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("gpui-shell acceptance: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .ok_or("usage: jit-gpui-shell-report INPUT OUTPUT")?;
    let output = args
        .next()
        .ok_or("usage: jit-gpui-shell-report INPUT OUTPUT")?;
    if args.next().is_some() {
        return Err("usage: jit-gpui-shell-report INPUT OUTPUT".into());
    }
    let report: Report = serde_json::from_slice(&fs::read(&input).map_err(err)?).map_err(err)?;
    let (markdown, pass) = validate_and_render(&report)?;
    if let Some(parent) = std::path::Path::new(&output).parent() {
        fs::create_dir_all(parent).map_err(err)?;
    }
    fs::write(output, markdown).map_err(err)?;
    if pass {
        Ok(())
    } else {
        Err("acceptance gates failed; see the generated Markdown report".into())
    }
}

fn validate_and_render(report: &Report) -> Result<(String, bool), String> {
    if report.schema != "gpui-shell-jit-v1" {
        return Err("unsupported schema; expected gpui-shell-jit-v1".into());
    }
    if report.policy.warmup_processes < 5
        || report.policy.paired_processes != REQUIRED_SAMPLES
        || report.policy.bootstrap_resamples < REQUIRED_BOOTSTRAPS
    {
        return Err(
            "sampling policy requires >=5 warmups, 30 pairs, and >=10000 bootstraps".into(),
        );
    }
    if report.provenance.shell_revision.len() != 40
        || report.provenance.rquickjs_revision.len() != 40
        || report.provenance.target_triple.split('-').count() < 3
        || report.provenance.cpu_affinity.is_empty()
        || report.provenance.command.is_empty()
        || ((report.provenance.shell_dirty || report.provenance.rquickjs_dirty)
            && (!is_sha256(&report.provenance.test_binary_sha256)
                || !is_sha256(&report.provenance.integration_patch_sha256)))
    {
        return Err("incomplete or dirty provenance".into());
    }
    validate_lifecycle(&report.lifecycle)?;
    if report.workloads.is_empty() {
        return Err("no real gpui-shell workloads supplied".into());
    }

    let mut markdown = format!(
        "# gpui-shell JIT acceptance\n\nShell `{}`, rquickjs `{}`, target `{}`, CPU affinity `{}`. {} paired fresh processes after {} discarded warmups.\n\n| workload | steady-state speedup CI | P99 regression CI | native entries | fallback | status |\n|---|---:|---:|---:|---:|---|\n",
        report.provenance.shell_revision,
        report.provenance.rquickjs_revision,
        report.provenance.target_triple,
        report.provenance.cpu_affinity,
        report.policy.paired_processes,
        report.policy.warmup_processes,
    );
    let mut suitable = 0;
    let mut suitable_pass = 0;
    let mut workloads_pass = true;
    let mut diagnostics = String::new();
    for workload in &report.workloads {
        validate_pairs(&workload.name, &workload.interpreter, &workload.automatic)?;
        let speedup = paired_bootstrap(&workload.interpreter, &workload.automatic, |i, a| {
            i.steady_state_ns as f64 / a.steady_state_ns as f64
        });
        let tail = paired_bootstrap(&workload.interpreter, &workload.automatic, |i, a| {
            a.p99_script_render_ns as f64 / i.p99_script_render_ns as f64
        });
        let native = workload
            .automatic
            .iter()
            .map(|x| x.native_entries)
            .sum::<u64>();
        let every_sample_native = workload.automatic.iter().all(|x| x.native_entries > 0);
        let fallback = workload
            .automatic
            .iter()
            .map(|x| x.fallback_count)
            .sum::<u64>();
        let suitable_gate = !workload.suitable_for_jit
            || (speedup[0] + f64::EPSILON * 8.0 >= 2.0 && every_sample_native);
        let regression_gate = !workload.regression_guard
            || (speedup[0] + f64::EPSILON * 8.0 >= 1.0 / 1.05 && tail[1] <= 1.05);
        let pass = suitable_gate && regression_gate;
        workloads_pass &= pass;
        if workload.suitable_for_jit {
            suitable += 1;
            suitable_pass += usize::from(pass);
        }
        markdown.push_str(&format!(
            "| {} | {:.2}x..{:.2}x | {:+.2}%..{:+.2}% | {} | {} | {} |\n",
            workload.name,
            speedup[0],
            speedup[1],
            (tail[0] - 1.0) * 100.0,
            (tail[1] - 1.0) * 100.0,
            native,
            fallback,
            if pass { "PASS" } else { "FAIL" },
        ));
        let sum = |field: fn(&Sample) -> u64| workload.automatic.iter().map(field).sum::<u64>();
        diagnostics.push_str(&format!(
            "- {}: installed={}, failures={} (unsupported={}, tier1={}, resource={}, cancelled={}, panics={}, invalid={}, install={}), native exits={}, OSR entries={}, deopts={}\n",
            workload.name,
            sum(|sample| sample.installed),
            sum(|sample| sample.compile_failures),
            sum(|sample| sample.unsupported_opcode_failures),
            sum(|sample| sample.tier1_rejections),
            sum(|sample| sample.resource_limit_failures),
            sum(|sample| sample.cancelled_compilations),
            sum(|sample| sample.compiler_panics),
            sum(|sample| sample.invalid_artifacts),
            sum(|sample| sample.install_failures),
            sum(|sample| sample.native_exits),
            sum(|sample| sample.osr_entries),
            sum(|sample| sample.deopts),
        ));
    }
    let first = lifecycle_ci(&report.lifecycle, |x| x.first_window_ns);
    let reload = lifecycle_ci(&report.lifecycle, |x| x.hot_reload_ns);
    let lifecycle_pass = first[1] <= 1.05 && reload[1] <= 1.05;
    let all_pass = suitable > 0 && suitable == suitable_pass && workloads_pass && lifecycle_pass;
    markdown.push_str(&format!(
        "\nDiagnostics across automatic samples:\n\n{}\nLifecycle regression CIs: first window {:+.2}%..{:+.2}%; hot reload {:+.2}%..{:+.2}%. Snapshots and script-render counts match pairwise.\n\nOverall: **{}**.\n",
        diagnostics,
        (first[0] - 1.0) * 100.0,
        (first[1] - 1.0) * 100.0,
        (reload[0] - 1.0) * 100.0,
        (reload[1] - 1.0) * 100.0,
        if all_pass { "PASS" } else { "FAIL" },
    ));
    Ok((markdown, all_pass))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_pairs(name: &str, interpreter: &[Sample], automatic: &[Sample]) -> Result<(), String> {
    if interpreter.len() != REQUIRED_SAMPLES || automatic.len() != REQUIRED_SAMPLES {
        return Err(format!("{name}: expected 30 samples per mode"));
    }
    for (index, (i, a)) in interpreter.iter().zip(automatic).enumerate() {
        if i.pair_index != index || a.pair_index != index {
            return Err(format!("{name}: invalid pair index at {index}"));
        }
        if i.steady_state_ns == 0
            || i.p99_script_render_ns == 0
            || a.steady_state_ns == 0
            || a.p99_script_render_ns == 0
        {
            return Err(format!("{name}: zero timing at pair {index}"));
        }
        if i.checksum != a.checksum
            || i.snapshot_sha256 != a.snapshot_sha256
            || i.script_renders != a.script_renders
        {
            return Err(format!("{name}: semantic mismatch at pair {index}"));
        }
        if i.native_enabled || !a.native_enabled {
            return Err(format!("{name}: invalid runtime mode at pair {index}"));
        }
        if i.native_entries != 0 || i.installed != 0 || i.compile_failures != 0 {
            return Err(format!(
                "{name}: interpreter recorded JIT activity at pair {index}"
            ));
        }
        let categorized_failures = a
            .unsupported_opcode_failures
            .saturating_add(a.tier1_rejections)
            .saturating_add(a.resource_limit_failures)
            .saturating_add(a.cancelled_compilations)
            .saturating_add(a.compiler_panics)
            .saturating_add(a.invalid_artifacts)
            .saturating_add(a.install_failures);
        if a.compile_failures > 0 && categorized_failures == 0 {
            return Err(format!(
                "{name}: uncategorized compilation failures at pair {index}"
            ));
        }
    }
    Ok(())
}

fn validate_lifecycle(lifecycle: &Lifecycle) -> Result<(), String> {
    if lifecycle.interpreter.len() != REQUIRED_SAMPLES
        || lifecycle.automatic.len() != REQUIRED_SAMPLES
    {
        return Err("lifecycle: expected 30 samples per mode".into());
    }
    for (index, (i, a)) in lifecycle
        .interpreter
        .iter()
        .zip(&lifecycle.automatic)
        .enumerate()
    {
        if i.pair_index != index
            || a.pair_index != index
            || i.first_window_ns == 0
            || a.first_window_ns == 0
            || i.hot_reload_ns == 0
            || a.hot_reload_ns == 0
            || i.reload_observations.len() != 5
            || a.reload_observations.len() != 5
        {
            return Err(format!("lifecycle: invalid sample {index}"));
        }
        if i.snapshot_sha256 != a.snapshot_sha256 || i.script_renders != a.script_renders {
            return Err(format!("lifecycle: semantic mismatch at pair {index}"));
        }
    }
    Ok(())
}

fn lifecycle_ci(lifecycle: &Lifecycle, field: impl Fn(&LifecycleSample) -> u64 + Copy) -> [f64; 2] {
    paired_bootstrap(&lifecycle.interpreter, &lifecycle.automatic, |i, a| {
        field(a) as f64 / field(i) as f64
    })
}

fn paired_bootstrap<T>(left: &[T], right: &[T], ratio: impl Fn(&T, &T) -> f64) -> [f64; 2] {
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut values = Vec::with_capacity(REQUIRED_BOOTSTRAPS);
    for _ in 0..REQUIRED_BOOTSTRAPS {
        let mut logs = 0.0;
        for _ in 0..left.len() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let index = state as usize % left.len();
            logs += ratio(&left[index], &right[index]).ln();
        }
        values.push((logs / left.len() as f64).exp());
    }
    values.sort_by(f64::total_cmp);
    [values[249], values[9_749]]
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(speedup: u64, automatic_native_entries: u64) -> Report {
        let samples = |time, native_enabled, native_entries| {
            (0..REQUIRED_SAMPLES)
                .map(|pair_index| Sample {
                    pair_index,
                    steady_state_ns: time,
                    p99_script_render_ns: 100,
                    checksum: "same".into(),
                    snapshot_sha256: "snapshot".into(),
                    script_renders: 1,
                    native_enabled,
                    native_entries,
                    fallback_count: 0,
                    installed: native_entries,
                    compile_failures: 0,
                    unsupported_opcode_failures: 0,
                    tier1_rejections: 0,
                    resource_limit_failures: 0,
                    cancelled_compilations: 0,
                    compiler_panics: 0,
                    invalid_artifacts: 0,
                    install_failures: 0,
                    native_exits: native_entries,
                    osr_entries: 0,
                    deopts: 0,
                })
                .collect()
        };
        let lifecycle = |time| {
            (0..REQUIRED_SAMPLES)
                .map(|pair_index| LifecycleSample {
                    pair_index,
                    first_window_ns: time,
                    hot_reload_ns: time,
                    snapshot_sha256: "snapshot".into(),
                    script_renders: 1,
                    reload_observations: vec![serde_json::json!({}); 5],
                })
                .collect()
        };
        Report {
            schema: "gpui-shell-jit-v1".into(),
            provenance: Provenance {
                shell_revision: "a".repeat(40),
                rquickjs_revision: "b".repeat(40),
                shell_dirty: false,
                rquickjs_dirty: false,
                test_binary_sha256: "0".repeat(64),
                integration_patch_sha256: "0".repeat(64),
                cpu_affinity: "0".into(),
                target_triple: "x86_64-unknown-linux-gnu".into(),
                command: vec!["real-shell-benchmark".into()],
            },
            policy: Policy {
                warmup_processes: 5,
                paired_processes: 30,
                bootstrap_resamples: 10_000,
            },
            workloads: vec![Workload {
                name: "real panel".into(),
                suitable_for_jit: true,
                regression_guard: true,
                interpreter: samples(100 * speedup, false, 0),
                automatic: samples(100, true, automatic_native_entries),
            }],
            lifecycle: Lifecycle {
                interpreter: lifecycle(100),
                automatic: lifecycle(100),
            },
        }
    }

    #[test]
    fn paired_bootstrap_preserves_constant_ratio() {
        let left = vec![200_u64; 30];
        let right = vec![100_u64; 30];
        let ci = paired_bootstrap(&left, &right, |a, b| *a as f64 / *b as f64);
        assert!((ci[0] - 2.0).abs() < 1e-12);
        assert!((ci[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn real_semantic_pairs_and_two_x_native_speedup_pass() {
        let (markdown, pass) = validate_and_render(&report(2, 1)).unwrap();
        assert!(pass);
        assert!(markdown.contains("Overall: **PASS**"));
    }

    #[test]
    fn fixture_like_timing_without_native_entries_fails() {
        let (markdown, pass) = validate_and_render(&report(3, 0)).unwrap();
        assert!(!pass);
        assert!(markdown.contains("Overall: **FAIL**"));
    }

    #[test]
    fn threshold_only_runtime_is_rejected_as_an_interpreter_baseline() {
        let mut report = report(2, 1);
        report.workloads[0].interpreter[0].native_enabled = true;

        let error = validate_and_render(&report).unwrap_err();
        assert!(error.contains("invalid runtime mode at pair 0"), "{error}");
    }

    #[test]
    fn aggregate_failure_without_a_category_is_rejected() {
        let mut report = report(2, 1);
        report.workloads[0].automatic[0].compile_failures = 1;

        let error = validate_and_render(&report).unwrap_err();
        assert!(
            error.contains("uncategorized compilation failures at pair 0"),
            "{error}"
        );
    }

    #[test]
    fn guarded_host_tail_regression_blocks_overall_acceptance() {
        let mut report = report(2, 1);
        let mut guarded = report.workloads[0].clone();
        guarded.name = "host panel".into();
        guarded.suitable_for_jit = false;
        guarded.automatic.iter_mut().for_each(|sample| {
            sample.p99_script_render_ns = 106;
        });
        report.workloads.push(guarded);

        let (markdown, pass) = validate_and_render(&report).unwrap();
        assert!(!pass);
        assert!(markdown.contains("host panel"));
        assert!(markdown.contains("Overall: **FAIL**"));
    }
}
