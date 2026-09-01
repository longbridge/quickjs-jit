function recursiveSum(n) { return n <= 0 ? 0 : n + recursiveSum(n - 1); }
function call4(fn, value) { return fn(value); }
function call3(fn, value) { return call4(fn, value); }
function call2(fn, value) { return call3(fn, value); }
function call1(fn, value) { return call2(fn, value); }
function workload(iterations, seed) {
  let captured = seed;
  const update = value => (captured = (captured + value) | 0);
  for (let i = 0; i < iterations; i++) call1(update, recursiveSum(i & 7));
  return captured;
}
