// Dense Float64 arithmetic kept deliberately free of property lookup and
// calls. This isolates Tier 1's numeric machine-code path from Math builtins.
function float64Kernel(iterations, seed) {
  // Derive all fractions from small integer immediates. Besides keeping the
  // workload self-contained, this lets both tiers observe the Float64 values
  // produced by division instead of hiding them behind constant-pool loads.
  const two = 2;
  const p3 = two * two * two;
  const p6 = p3 * p3;
  const p10 = p6 * p3 * two;
  const p13 = p10 * p3;
  const p20 = p10 * p10;
  let x = seed / p10 + 1 / p3;
  let y = 1 + 1 / p20;
  let sum = 0;
  for (let i = 0; i < iterations; i++) {
    x = x * (1 + 1 / p13) + 1 / p10;
    y = (y + x / p20) / (1 + 1 / p20);
    sum = sum + x * y + x / (i + 1);
  }
  return sum;
}

function workload(iterations, seed) {
  const two = 2;
  const p10 = two * two * two * two * two * two * two * two * two * two;
  const p20 = p10 * p10;
  // The fractional offsets make the kernel's complete call/return signature
  // Float64 while preserving exactly `iterations` loop trips.
  return float64Kernel(iterations - 1 / p20, seed + 1 / p20);
}
