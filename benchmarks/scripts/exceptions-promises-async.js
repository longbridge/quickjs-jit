async function asyncStep(value) { return (await Promise.resolve(value + 1)) * 3; }
async function workload(iterations, seed) {
  let caught = 0;
  for (let i = 0; i < iterations; i++) {
    try { if ((i & 15) === 0) throw i; } catch (value) { caught += value; }
  }
  let continued = seed;
  for (let i = 0; i < 64; i++) continued = await asyncStep(continued & 0xffff);
  return caught + ":" + continued;
}
