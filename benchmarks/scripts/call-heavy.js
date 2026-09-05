// Focused monomorphic-call kernel with stable target, arity, and value types.
function increment(value, delta) {
  return value + delta;
}

globalThis.workloadArgument = increment;
// The leaf, initial caller, and direct-edge caller refresh must all publish
// before the harness starts steady-state timing.
globalThis.tier1ReadyInstalls = 3;
// Forced Tier 2 needs both baseline and both optimizing publications.
globalThis.tier2ReadyInstalls = 4;

function workload(iterations, seed, target) {
  let value = seed;
  let delta = 0;
  for (let i = 0; i < iterations; i++) {
    value = target(value, delta);
    delta++;
    if (delta < 8) {
      // Keep the repeating argument distribution without an unsupported
      // bitwise/modulo opcode obscuring the call-path measurement.
    } else {
      delta = 0;
    }
  }
  return value;
}
