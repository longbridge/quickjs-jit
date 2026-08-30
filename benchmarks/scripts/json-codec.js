function workload(iterations, seed) {
  const payload = { name: "rquickjs", enabled: true, values: [1, 2.5, null, "four"], nested: { seed, tags: ["jit", "quickjs", "benchmark"] } };
  let total = 0;
  let last = "";
  for (let i = 0; i < iterations; i++) {
    payload.nested.seed = i + seed;
    last = JSON.stringify(payload);
    const decoded = JSON.parse(last);
    total += decoded.values.length + decoded.nested.tags.length + decoded.nested.seed;
  }
  return total + ":" + last.length;
}
