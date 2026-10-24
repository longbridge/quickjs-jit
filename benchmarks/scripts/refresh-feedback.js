// Several small builders populate call feedback while returning fresh objects.
// Function definitions stay outside the timed workload in every engine.
var builders = [];
for (var i = 0; i < 16; i++) {
  builders.push(new Function('value', 'return { value: value };'));
}
globalThis.workloadArgument = builders;

function workload(iterations, seed, make) {
  var sum = seed;
  for (var i = 0; i < iterations; i++) {
    sum += make[i & 15](i).value;
  }
  return sum;
}
