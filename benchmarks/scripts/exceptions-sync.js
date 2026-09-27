// Synchronous exception regions: a primitive throw caught every 16th
// iteration, a finally block on every iteration, and a helper-raised
// TypeError (property read of null) on one of every 256 iterations.
function workload(iterations, seed) {
  const table = [{ w: 1 }, { w: 2 }, null, { w: 3 }];
  let caught = 0;
  let total = seed | 0;
  let finals = 0;
  let missing = 0;
  for (let i = 0; i < iterations; i++) {
    try {
      if ((i & 15) === 0) throw i;
      total = (total + i) | 0;
    } catch (value) {
      caught += value;
    } finally {
      finals++;
    }
    if ((i & 63) === 1) {
      try {
        total = (total + table[(i >> 6) & 3].w) | 0;
      } catch (error) {
        missing++;
      }
    }
  }
  return caught + ':' + total + ':' + finals + ':' + missing;
}
