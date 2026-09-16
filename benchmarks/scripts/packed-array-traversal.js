// Construct and populate the stable input before the harness starts timing.
const packedTraversalValues = new Array(2000);
for (let i = 0; i < packedTraversalValues.length; i++) {
  packedTraversalValues[i] = (i * 17) & 0xffff;
}
globalThis.workloadArgument = packedTraversalValues;

function workload(iterations, seed, values) {
  let sum = seed;
  for (let i = 0; i < values.length; i++) {
    sum = (sum + values[i]) | 0;
  }
  return sum;
}
