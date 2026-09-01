// Focused monomorphic-property kernel: one stable object shape and fixed keys.
// Supplying the receiver as a benchmark argument keeps object-literal setup
// out of the measured function and lets Tier 1 compile the property loop.
globalThis.workloadArgument = { x: 0, y: 1 };

function workload(iterations, seed, point) {
  point.x = seed;
  point.y = 1;
  let toggle = 0;
  for (let i = 0; i < iterations; i++) {
    point.x = point.x + point.y;
    point.y = point.y + toggle;
    toggle = 1 - toggle;
  }
  return point.x + point.y;
}
