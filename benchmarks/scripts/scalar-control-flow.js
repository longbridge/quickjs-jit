// Both branch exits establish numeric x/y values. Their Phi results feed the
// same multiply, exercising CFG fact merging under production automatic tiering.
function workload(iterations, seed) {
  let sum = seed;
  for (let i = seed; i < iterations; i++) {
    let x, y;
    if (i & 1) {
      x = i + seed;
      y = i - seed;
    } else {
      x = i - seed;
      y = i + seed;
    }
    sum = (sum + ((x * y) & 1023)) | 0;
  }
  return sum;
}
