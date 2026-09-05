// A Bool parameter excludes the numeric direct-call signature. Keep the
// callee short so this measures the generic CALL/entry boundary in a batch.
function incrementIf(value, enabled) {
  if (enabled) return value + 1;
  return value;
}

globalThis.workloadArgument = incrementIf;
globalThis.tier1ReadyInstalls = 2;

function workload(iterations, seed, target) {
  let value = seed;
  for (let i = 0; i < iterations; i++) {
    value = target(value, true);
  }
  return value;
}
