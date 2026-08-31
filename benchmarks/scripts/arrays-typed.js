// Keep each hot traversal in one callee.  `workload` deliberately also covers
// allocation and typed-array construction, neither of which is a Tier 1
// kernel.  Splitting an individual element access into a tiny callee would
// therefore enter and leave native code once per element from that
// interpreter-only orchestration frame.
function sumElements(values) {
  let sum = 0;
  for (let i = 0; i < values.length; i++) sum = (sum + values[i]) | 0;
  return sum;
}

function convertAndSum(ints, floats) {
  let mixed = 0.0;
  for (let i = 0; i < ints.length; i++) {
    floats[i] = ints[i] * 0.25 + 0.5;
    mixed += floats[i];
  }
  return mixed;
}

function workload(iterations, seed) {
  const values = [];
  for (let i = 0; i < iterations; i++) values.push((i * 17 + seed) & 0xffff);
  const sum = sumElements(values);
  const ints = new Int32Array(values);
  const floats = new Float64Array(iterations);
  const mixed = convertAndSum(ints, floats);
  return sum + ":" + mixed.toFixed(3) + ":" + values.length;
}
