function workload(iterations) {
  let value = 0;
  for (let i = 0; i < iterations; i++) value = i % 7 ? value + i : String(value).length;
  return String(value);
}
