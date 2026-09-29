// Construct and populate both stable views before the harness starts timing.
// Unlike float64array-traversal, both typed arrays reach the hot loop as
// arguments of one callee, and only the source's length bounds the loop: the
// destination is a second, simultaneously live typed receiver.
const typedConvertInts = new Int32Array(2000);
const typedConvertFloats = new Float64Array(2000);
for (let i = 0; i < typedConvertInts.length; i++) {
  typedConvertInts[i] = (i * 17) & 0xffff;
}
globalThis.workloadArgument = typedConvertInts;

function convertAndSum(ints, floats) {
  let mixed = 0.0;
  for (let i = 0; i < ints.length; i++) {
    floats[i] = ints[i] * 0.25 + 0.5;
    mixed += floats[i];
  }
  return mixed;
}

function workload(iterations, seed, ints) {
  return convertAndSum(ints, typedConvertFloats) + seed;
}
