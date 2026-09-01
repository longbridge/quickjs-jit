// Deliberately call-heavy recursion probe; currently expected to fail closed.
function fibonacci(n) {
  if (n < 2) return n;
  return fibonacci(n - 1) + fibonacci(n - 2);
}

function workload(_iterations, seed) {
  return fibonacci(20) + seed;
}
