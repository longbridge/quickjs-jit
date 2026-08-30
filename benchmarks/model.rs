use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BenchmarkFile {
    pub schema: String,
    pub provenance: Provenance,
    pub policy: SamplingPolicy,
    pub modes: Vec<ModeResult>,
    pub exclusions: Vec<Exclusion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Provenance {
    pub source_revision: String,
    pub quickjs_revision: String,
    pub target: String,
    pub os: String,
    pub kernel: String,
    pub cpu: String,
    pub power_mode: String,
    pub rustc: String,
    pub llvm: String,
    pub executable_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SamplingPolicy {
    pub latency_warmups: u32,
    pub latency_processes: u32,
    pub throughput_windows: u32,
    pub throughput_window_ns: u64,
    pub bootstrap_resamples: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModeResult {
    pub mode: String,
    pub workloads: Vec<WorkloadResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkloadResult {
    pub name: String,
    pub group: String,
    pub designated_kernel: bool,
    pub raw_latency_ns: Vec<u64>,
    pub raw_throughput_ops: Vec<u64>,
    pub median_ns: u64,
    pub mad_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub ci95_ns: [u64; 2],
    pub checksum: String,
    pub native_entries: u64,
    pub native_exits: u64,
    pub fallback_count: u64,
    pub retry_count: u64,
    pub tier1_entries: u64,
    pub tier2_entries: u64,
    pub osr_entries: u64,
    pub compile_ns: u64,
    pub install_ns: u64,
    pub break_even_executions: Option<u64>,
    pub peak_rss_bytes: u64,
    pub code_bytes: u64,
    pub metadata_bytes: u64,
    pub peak_compiler_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Exclusion {
    pub suite: String,
    pub test: String,
    pub reason: String,
}

#[allow(dead_code)] // Used by the runner binary; the reporter shares this schema module.
pub fn summarize(mut raw: Vec<u64>) -> (u64, u64, u64, u64, [u64; 2]) {
    raw.sort_unstable();
    let median = quantile(&raw, 0.5);
    let mut deviations = raw.iter().map(|x| x.abs_diff(median)).collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad = quantile(&deviations, 0.5);
    let ci = bootstrap_median_ci(&raw, 10_000);
    (median, mad, quantile(&raw, 0.95), quantile(&raw, 0.99), ci)
}

#[allow(dead_code)]
pub fn quantile(values: &[u64], quantile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) as f64 * quantile).ceil() as usize;
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
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *slot = values[(state as usize) % values.len()];
        }
        sample.sort_unstable();
        medians.push(quantile(&sample, 0.5));
    }
    medians.sort_unstable();
    [quantile(&medians, 0.025), quantile(&medians, 0.975)]
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
}
