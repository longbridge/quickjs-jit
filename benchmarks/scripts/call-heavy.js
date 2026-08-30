// Focused monomorphic-call kernel with stable target, arity, and value types.
function increment(value, delta) {
  return value + delta;
}

globalThis.workloadArgument = increment;

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
