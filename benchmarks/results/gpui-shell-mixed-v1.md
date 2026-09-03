# gpui-shell JIT acceptance

Shell `c0b989691707d5c18d973cf9cf805bbcadde37b5`, rquickjs `034c9f04f80ef0464ee2589dbb202608ac090e4d`, target `x86_64-unknown-linux-gnu`, CPU affinity `0`. 30 paired fresh processes after 5 discarded warmups.

| workload | steady-state speedup CI | P99 speed CI | native entries | fallback | status |
|---|---:|---:|---:|---:|---|
| realistic 443-node host-heavy panel | 0.99x..0.99x | 0.99x..1.00x | 0 | 0 | PASS |
| render-driven numeric layout checksum | 15.34x..18.33x | 10.73x..15.24x | 7028 | 0 | PASS |
| market compute, sort, aggregate, and visible list | 2.32x..2.34x | 2.20x..2.27x | 732467 | 0 | PASS |

Diagnostics across automatic samples:

- realistic 443-node host-heavy panel: installed=0, failures=420 (unsupported=0, tier1=420, resource=0, cancelled=0, panics=0, invalid=0, install=0), native exits=0, OSR entries=0, deopts=0
- render-driven numeric layout checksum: installed=60, failures=210 (unsupported=0, tier1=210, resource=0, cancelled=0, panics=0, invalid=0, install=0), native exits=7028, OSR entries=27, deopts=0
- market compute, sort, aggregate, and visible list: installed=90, failures=390 (unsupported=0, tier1=270, resource=0, cancelled=0, panics=0, invalid=120, install=0), native exits=732467, OSR entries=0, deopts=0

Lifecycle speed CIs: first window 0.99x..1.00x; hot reload 0.99x..1.00x. Snapshots and script-render counts match pairwise.

Overall: **PASS**.
