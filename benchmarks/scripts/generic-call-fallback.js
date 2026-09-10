// Observable mutation keeps this callee outside the pure direct-leaf ABI.
// Keep the original generic-call-entry probe unchanged for paired history.
function incrementAndRecord(value, enabled, state) {
  state.calls = state.calls + 1;
  if (enabled) return value + 1;
  return value;
}

globalThis.workloadArgument = { target: incrementAndRecord, calls: 0 };

function workload(iterations, seed, state) {
  state.calls = 0;
  const target = state.target;
  let value = seed;
  for (let i = 0; i < iterations; i++) {
    value = target(value, true, state);
  }
  // Consume both the return values and the mutation; reset on every call.
  return value + state.calls;
}
