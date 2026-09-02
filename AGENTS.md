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

Express performance comparisons in the direction of speed, relative to the
named baseline. Prefer `1.25x the baseline speed` or `25% faster`; `1.00x` means
equal speed, values above `1.00x` are faster, and values below `1.00x` are
slower. Do not present improvements as negative latency regressions because the
sign is easy to misread. When a confidence interval crosses parity, say that
the result is statistically tied and give the plain-language range, for example
`between 3% slower and 5% faster`. Preserve whether a number measures latency,
throughput, or speedup, and mathematically convert latency changes before
describing them as speed changes.
