function workload(iterations) {
  const values = []; let sum = 0;
  for (let i = 0; i < iterations; i++) values.push((i * 17) & 1023);
  for (const value of values) sum += value;
  return String(sum);
}
