# gpui-shell JIT acceptance

Shell `3d01ba11740b5c5b4ef6a9ef4b6aea2157b78192`, rquickjs `71d6c018c3b6564eed79ce6d9e37dceb10e3163e`, target `x86_64-unknown-linux-gnu`, CPU affinity `0`. 30 paired fresh processes after 5 discarded warmups.

| workload | steady-state speedup CI | P99 regression CI | native entries | fallback | status |
|---|---:|---:|---:|---:|---|
| realistic 443-node host-heavy panel | 0.99x..1.00x | -7.21%..+2.74% | 0 | 0 | PASS |
| render-driven numeric layout checksum | 39.82x..40.19x | -97.36%..-97.19% | 16835 | 0 | PASS |

Lifecycle regression CIs: first window -0.77%..+0.44%; hot reload -3.37%..+0.84%. Snapshots and script-render counts match pairwise.

Overall: **PASS**.
