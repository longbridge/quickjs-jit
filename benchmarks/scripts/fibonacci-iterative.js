// Bounded Int32 loop/Phi probe. Batch inside JavaScript so the call boundary
// does not dominate the compute evidence.
// fib(40) is 102334155 and fits in Int32.
function workload(iterations, seed) {
  let result = seed;
  for (let batch = seed; batch < iterations; batch++) {
    let a = seed;
    let b = 1;
    for (let i = seed; i < 40; i++) {
      const next = a + b;
      a = b;
      b = next;
    }
    result = a;
  }
  return result;
}
