# gpui-shell JIT acceptance

Shell `3d01ba11740b5c5b4ef6a9ef4b6aea2157b78192`, rquickjs `fa4e7b0d27b36e385990449ed04e03f4fb9b4b7d`, target `x86_64-unknown-linux-gnu`. 30 paired fresh processes after 5 discarded warmups.

| workload | steady-state speedup CI | P99 regression CI | native entries | fallback | status |
|---|---:|---:|---:|---:|---|
| realistic 443-node host-heavy panel | 0.98x..0.99x | -4.83%..+5.70% | 0 | 0 | FAIL |
| render-driven numeric layout checksum | 37.68x..39.43x | -97.17%..-97.00% | 3379 | 0 | PASS |

Lifecycle regression CIs: first window +67.13%..+93.31%; hot reload +462.94%..+508.99%. Snapshots and script-render counts match pairwise.

Overall: **FAIL**.
