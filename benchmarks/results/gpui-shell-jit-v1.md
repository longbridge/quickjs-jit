# gpui-shell JIT acceptance

Shell `3d01ba11740b5c5b4ef6a9ef4b6aea2157b78192`, rquickjs `0563f1af52496378389b046af3cc92d04c933e3e`, target `x86_64-unknown-linux-gnu`. 30 paired fresh processes after 5 discarded warmups.

| workload | steady-state speedup CI | P99 regression CI | native entries | fallback | status |
|---|---:|---:|---:|---:|---|
| realistic 443-node host-heavy panel | 0.93x..1.10x | -30.26%..+6.83% | 0 | 0 | FAIL |
| render-driven numeric layout checksum | 33.49x..40.37x | -97.47%..-96.90% | 3336 | 0 | PASS |

Lifecycle regression CIs: first window -14.10%..+10.50%; hot reload -3.70%..+12.76%. Snapshots and script-render counts match pairwise.

Overall: **FAIL**.
