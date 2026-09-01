// Focused scalar-loop kernel: integer induction, addition, and one backedge.
function workload(iterations, seed) {
  let sum = seed;
  for (let i = seed; i < iterations; i++) {
    sum = sum + i;
  }
  return sum;
}
