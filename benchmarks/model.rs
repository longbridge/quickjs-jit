use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkFile {
    pub schema: String,
    pub provenance: Provenance,
    pub policy: SamplingPolicy,
    pub modes: Vec<ModeResult>,
    pub exclusions: Vec<Exclusion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_revision: String,
    pub quickjs_revision: String,
    pub source_dirty: bool,
    pub command: Vec<String>,
    pub target: String,
    pub target_triple: String,
    pub os: String,
    pub kernel: String,
    pub cpu: String,
    pub power_mode: String,
    pub rustc: String,
    pub llvm: String,
    pub executable_bytes: u64,
    pub stripped_jit_bytes: u64,
    pub stripped_no_jit_bytes: u64,
    pub stripped_jit_delta_bytes: i64,
    pub schema_sha256: String,
    pub suites_lock_sha256: String,
    pub bun_version: Option<String>,
    pub bun_path: Option<String>,
    pub bun_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SamplingPolicy {
    pub latency_warmups: u32,
    pub latency_processes: u32,
    pub throughput_windows: u32,
    pub throughput_window_ns: u64,
    pub bootstrap_resamples: u32,
    pub pairing: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModeResult {
    pub mode: String,
    pub workloads: Vec<WorkloadResult>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseTiming {
    pub total_ns: u64,
    pub runtime_create_ns: u64,
    pub jit_attach_ns: u64,
    pub context_create_ns: u64,
    pub definition_eval_ns: u64,
    pub first_eval_ns: u64,
    pub threshold_crossing_ns: u64,
    pub compile_ns: u64,
    pub install_ns: u64,
    pub osr_ns: u64,
    pub steady_state_ns: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampleEvidence {
    #[serde(default)]
    pub protocol: Option<ProtocolEvidence>,
    pub pair_index: u32,
    pub elapsed_ns: u64,
    pub checksum: String,
    pub native_entries: Option<u64>,
    #[serde(default)]
    pub native_acquisitions: Option<u64>,
    pub native_exits: Option<u64>,
    pub fallback_count: Option<u64>,
    pub retry_count: Option<u64>,
    pub tier1_entries: Option<u64>,
    pub tier2_entries: Option<u64>,
    pub deopt_count: Option<u64>,
    pub osr_attempts: Option<u64>,
    pub profitability_evaluations: Option<u64>,
    pub profitability_approved: Option<u64>,
    pub profitability_rejected: Option<u64>,
    pub benefit_recordings: Option<u64>,
    pub measured_benefit_ns: Option<u64>,
    pub opcode_fingerprint: Option<u64>,
    pub abi_fingerprint: Option<u64>,
    pub config_fingerprint: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub code_bytes: Option<u64>,
    pub metadata_bytes: Option<u64>,
    pub peak_compiler_bytes: Option<u64>,
    pub phases: PhaseTiming,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadResult {
    pub name: String,
    pub suite: String,
    pub group: String,
    pub designated_kernel: bool,
    pub samples: Vec<SampleEvidence>,
    pub raw_latency_ns: Vec<u64>,
    pub raw_throughput_ops: Vec<u64>,
    pub median_ns: u64,
    pub mad_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub ci95_ns: [u64; 2],
    pub compile_ns: u64,
    pub install_ns: u64,
    pub break_even_executions: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    pub suite: String,
    pub test: String,
    pub reason: String,
}

#[allow(dead_code)]
pub fn summarize(mut raw: Vec<u64>) -> (u64, u64, u64, u64, [u64; 2]) {
    raw.sort_unstable();
    let median = quantile(&raw, 0.5);
    let mut deviations = raw.iter().map(|x| x.abs_diff(median)).collect::<Vec<_>>();
    deviations.sort_unstable();
    let ci = bootstrap_median_ci(&raw, 10_000);
    (
        median,
        quantile(&deviations, 0.5),
        quantile(&raw, 0.95),
        quantile(&raw, 0.99),
        ci,
    )
}

pub fn quantile(values: &[u64], q: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) as f64 * q).ceil() as usize;
    values[index.min(values.len() - 1)]
}

#[allow(dead_code)]
pub fn quantile_f64(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let index = ((values.len() - 1) as f64 * q).ceil() as usize;
    values[index.min(values.len() - 1)]
}

#[allow(dead_code)]
pub fn bootstrap_median_ci(values: &[u64], resamples: usize) -> [u64; 2] {
    if values.is_empty() {
        return [0, 0];
    }
    let mut state = 0x6a09e667f3bcc909u64;
    let mut medians = Vec::with_capacity(resamples);
    let mut sample = vec![0; values.len()];
    for _ in 0..resamples {
        for slot in &mut sample {
            state = xorshift(state);
            *slot = values[(state as usize) % values.len()];
        }
        sample.sort_unstable();
        medians.push(quantile(&sample, 0.5));
    }
    medians.sort_unstable();
    [quantile(&medians, 0.025), quantile(&medians, 0.975)]
}

pub fn xorshift(mut state: u64) -> u64 {
    state ^= state << 13;
    state ^= state >> 7;
    state ^ (state << 17)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_preserves_dispersion_and_tail() {
        let (median, mad, p95, p99, ci) = summarize((1..=100).collect());
        assert_eq!(median, 51);
        assert!(mad >= 25);
        assert_eq!(p95, 96);
        assert_eq!(p99, 100);
        assert!(ci[0] <= median && median <= ci[1]);
    }

    #[test]
    fn timed_batch_summary_uses_the_median_and_counts_but_keeps_spikes() {
        // A single one-off stall (the old fixed-batch artifact) no longer
        // determines the reported latency, but it stays visible.
        let mut batches = vec![5_000u64; 16];
        batches[0] = 187_000;
        let (median, outliers) = timed_batch_summary(&batches);
        assert_eq!(median, 5_000);
        assert_eq!(outliers, 1);
        // Exactly 5x the median is not an outlier; above it is.
        batches[0] = 25_000;
        assert_eq!(timed_batch_summary(&batches).1, 0);
        batches[0] = 25_001;
        assert_eq!(timed_batch_summary(&batches).1, 1);
        // Even K uses the harness-wide upper median.
        assert_eq!(timed_batch_summary(&[1, 2, 3, 4]), (3, 0));
        assert_eq!(timed_batch_summary(&[]), (0, 0));
    }
}

/// Legacy timing protocol: 64 warmup batches, then one timed batch.
#[allow(dead_code)]
pub const PROTOCOL_FIXED_WARMUP_V2: &str = "shared-js-fixed-warmup-v2";
/// Current timing protocol: 64 warmup batches, then K consecutive timed
/// batches whose median becomes `elapsed_ns`.
#[allow(dead_code)]
pub const PROTOCOL_MULTIBATCH_V3: &str = "shared-js-multibatch-v3";
/// A timed batch slower than this multiple of the timed-batch median is
/// counted as an outlier. Outliers are never discarded.
#[allow(dead_code)]
pub const OUTLIER_RATIO: u64 = 5;

/// Protocol identity separates these fixed-warmup timings from historical v1 timing.
///
/// Fields after `readiness_diagnostic_ns` were added by
/// `shared-js-multibatch-v3`; they default to empty in v2 evidence.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolEvidence {
    pub name: String,
    pub script_sha256: String,
    pub driver_sha256: String,
    pub warmup_batches: u32,
    pub calls_per_batch: u32,
    pub warmup_batch_ns: Vec<u64>,
    pub fixed_metrics_before: Option<std::collections::BTreeMap<String, u64>>,
    pub fixed_metrics_after: Option<std::collections::BTreeMap<String, u64>>,
    pub readiness_diagnostic_ns: Option<u64>,
    /// Number of consecutive timed batches after the warmups (v3).
    #[serde(default)]
    pub timed_batches: u32,
    /// Raw latency of every timed batch, in execution order (v3).
    #[serde(default)]
    pub timed_batch_ns: Vec<u64>,
    /// Diagnostic only: the first timed batch, i.e. the single batch that
    /// `shared-js-fixed-warmup-v2` reported as `elapsed_ns` (v3).
    #[serde(default)]
    pub fixed_batch_ns: Option<u64>,
    /// Outlier rule: a timed batch above `outlier_ratio` times the median (v3).
    #[serde(default)]
    pub outlier_ratio: u64,
    /// Number of timed batches that exceed the outlier rule (v3).
    #[serde(default)]
    pub outlier_batches: u32,
    /// JIT metric snapshots around the whole timed window. They include the
    /// untimed checksum and poll steps between timed batches (v3).
    #[serde(default)]
    pub timed_metrics_before: Option<std::collections::BTreeMap<String, u64>>,
    #[serde(default)]
    pub timed_metrics_after: Option<std::collections::BTreeMap<String, u64>>,
    /// How Bun loaded its wrapper: `file` (v3 default) or `eval` (control) (v3).
    #[serde(default)]
    pub bun_launch: Option<String>,
    /// SHA-256 of the exact Bun wrapper module that was executed (v3).
    #[serde(default)]
    pub bun_wrapper_sha256: Option<String>,
}

/// Summary of K timed batches: (upper median, outlier count).
///
/// The median uses the same upper-quantile rule as the rest of the harness.
#[allow(dead_code)]
pub fn timed_batch_summary(batches: &[u64]) -> (u64, u32) {
    let mut sorted = batches.to_vec();
    sorted.sort_unstable();
    let median = quantile(&sorted, 0.5);
    let threshold = median.saturating_mul(OUTLIER_RATIO);
    let outliers = batches.iter().filter(|&&ns| ns > threshold).count();
    (median, u32::try_from(outliers).unwrap_or(u32::MAX))
}
