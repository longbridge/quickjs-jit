function workload(iterations, seed) {
  const map = new Map();
  const set = new Set();
  let big = BigInt(seed);
  for (let i = 0; i < iterations; i++) {
    const key = (i * 17) & 255;
    map.set(key, (map.get(key) || 0) + 1);
    set.add(key);
    big = (big * 33n + BigInt(key + 1)) & 0xffffffffffffffffn;
  }
  let count = 0;
  for (const value of map.values()) count += value;
  return count + ":" + set.size + ":" + big.toString(16);
}
