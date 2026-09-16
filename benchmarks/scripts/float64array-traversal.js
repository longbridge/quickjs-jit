// Construct and populate both stable buffers before the harness starts timing.
const float64TraversalInts = new Int32Array(2000);
const float64TraversalFloats = new Float64Array(2000);
for (let i = 0; i < float64TraversalInts.length; i++) {
  float64TraversalInts[i] = (i * 17) & 0xffff;
}
globalThis.workloadArgument = {
  ints: float64TraversalInts,
  floats: float64TraversalFloats,
};

function convertAndSum(iterations, seed, buffers) {
  const ints = buffers.ints;
  const floats = buffers.floats;
  let sum = seed;
  for (let i = 0; i < ints.length; i++) {
    floats[i] = ints[i] * 0.25 + 0.5;
    sum += floats[i];
  }
  return sum;
}
globalThis.workload = convertAndSum;
