// Local rquickjs-jit workload; not an extracted QuickJS benchmark.
function workload(n, seed) {
  let a = seed | 0, b = 1;
  for (let i = 0; i < n; i++) { const next = (a + b) | 0; a = b; b = next; }
  return a;
}
