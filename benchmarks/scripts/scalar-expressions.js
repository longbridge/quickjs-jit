// Repeated nested numeric expressions exercise semantic value identity across
// expression trees. Keep the sum within Int32 for the standard driver input;
// both engines consume the same full-loop result.
function workload(iterations, seed) {
  let sum = seed;
  for (let i = seed; i < iterations; i++) {
    const x = i & 7;
    sum += ((x + 1) * (x + 3)) + ((x + 1) * (x + 3));
  }
  return sum;
}
