// Locally adapted from the MIT-licensed QuickJS microbench style.
function workload(n, seed) {
  let value = seed | 0;
  for (let i = 0; i < n; i++) value = ((value << 5) ^ (value >>> 3) ^ i) | 0;
  return value;
}
