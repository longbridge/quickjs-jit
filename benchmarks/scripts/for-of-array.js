function workload(iterations) {
  let values = globalThis.forOfArrayValues;
  if (!values || values.length !== iterations) {
    values = [];
    for (let i = 0; i < iterations; i++) values.push((i * 17) & 1023);
    globalThis.forOfArrayValues = values;
  }
  let sum = 0;
  for (const value of values) sum += value;
  return String(sum);
}
