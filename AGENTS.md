# Agent Instructions

## Workflow for Small Fixes

For small fixes with clear scope and acceptance criteria, make the change
directly and verify it. Do not create design documents, specs, implementation
plans, or extra approval checkpoints. Use a full design and planning workflow
only when the change affects architecture or public APIs, spans multiple
subsystems, or has materially ambiguous requirements.

## Docker CI Verification

When a CI failure depends on Linux, a sanitizer, a target architecture, or
another environment unavailable on the host, use Docker to reproduce the CI
command locally when practical. Match the CI image, architecture, toolchain,
system packages, environment variables, Cargo flags, and test filter as closely
as possible. Mount the source tree read-only and put build outputs and caches in
Docker volumes so verification does not modify the working tree.

Treat the actual CI result as authoritative. On ARM hosts, tests run through an
x86_64 container use emulation and may differ from native x86_64 CI, especially
under memory, address, or thread sanitizers. Report those differences and do not
treat an emulation-only sanitizer failure as proof of a regression without
confirming it on native CI or a native target host.

## Performance Reporting

Every JIT optimization must include benchmark evidence comparing QuickJS,
Bun, and quickjs-jit. Use Bun's default configuration as the external
performance target, the previous JIT revision as the regression baseline,
and the same-version QuickJS interpreter as the native-profitability baseline.
Do not consider an optimization complete based only on JIT-versus-interpreter
or JIT-versus-previous-version results.

For every optimization, update the root README.md with the complete per-scenario
QuickJS, Bun, and quickjs-jit comparison data, including absolute timings and
speed ratios with confidence intervals. Include every benchmark scenario in
the matrix, including slower, unsupported, fallback-only, and inconclusive
results; do not select only favorable kernels or substitute a link to a report
for the full table. Use production automatic tiering for the quickjs-jit column;
keep forced tiers as explicitly labeled diagnostics. Add a representative
benchmark for an optimized path missing from the matrix.

Comparisons must use equivalent inputs, result consumption, warmup policies,
and timed boundaries. Record source revisions, engine versions/flags, host,
sample counts, units, and raw-evidence links. Retain paired before/after
evidence for the optimized paths and controls, and refresh the complete
three-engine README matrix for the final optimization candidate. Clearly label
historical data with its measured revision; missing new measurements remain
pending, not implied current results. Host-specific workloads need an equivalent
pure-JavaScript Bun comparison in addition to their actual host regression tests;
never divide Bun kernel time by a host-inclusive snapshot timing.

Express performance comparisons in the direction of speed, relative to the
named baseline. Prefer `1.25x the baseline speed` or `25% faster`; `1.00x` means
equal speed, values above `1.00x` are faster, and values below `1.00x` are
slower. Do not present improvements as negative latency regressions because the
sign is easy to misread. When a confidence interval crosses parity, say that
the result is statistically tied and give the plain-language range, for example
`between 3% slower and 5% faster`. Preserve whether a number measures latency,
throughput, or speedup, and mathematically convert latency changes before
describing them as speed changes.
