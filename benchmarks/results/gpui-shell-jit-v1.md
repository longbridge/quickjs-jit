# gpui-shell JIT acceptance

Shell `3d01ba11740b5c5b4ef6a9ef4b6aea2157b78192`, rquickjs `71d6c018c3b6564eed79ce6d9e37dceb10e3163e`, target `x86_64-unknown-linux-gnu`, CPU affinity `0`. 30 paired fresh processes after 5 discarded warmups.

| workload | steady-state speedup CI | P99 speed CI | native entries | fallback | status |
|---|---:|---:|---:|---:|---|
| realistic 443-node host-heavy panel | 0.99x..1.00x | 0.97x..1.08x | 0 | 0 | PASS |
| render-driven numeric layout checksum | 39.82x..40.19x | 35.59x..37.88x | 16835 | 0 | PASS |

Lifecycle speed CIs: first window 1.00x..1.01x; hot reload 0.99x..1.03x. Snapshots and script-render counts match pairwise.

Overall: **PASS**.
