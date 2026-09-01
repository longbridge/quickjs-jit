function workload(iterations) {
  const add = x => y => (x + y) | 0; let sum = 0;
  for (let i = 0; i < iterations; i++) sum = add(sum)(i & 31);
  return String(sum);
}
