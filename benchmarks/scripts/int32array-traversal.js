// Construct and populate the stable input before the harness starts timing.
const int32TraversalValues = new Int32Array(2000);
for (let i = 0; i < int32TraversalValues.length; i++) {
  int32TraversalValues[i] = (i * 17) & 0xffff;
}
globalThis.workloadArgument = int32TraversalValues;

function workload(iterations, seed, values) {
  let sum = seed;
  for (let i = 0; i < values.length; i++) {
    sum = (sum + values[i]) | 0;
  }
  return sum;
}
