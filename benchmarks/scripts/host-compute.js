// Pure JS adaptation of gpui-shell COMPUTE_TEMPLATE at
// c0b989691707d5c18d973cf9cf805bbcadde37b5. Keep layoutKernel identical
// to the archived host source, including the overwritten batch checksum:
// changing it to an accumulator would measure a different optimization problem.
// One workload call is one render's computation and text creation. GPUI
// allocation, snapshot construction, and debug-tree validation are excluded.
function layoutKernel(batches, seed) {
  let checksum = seed;
  for (let batch = 0; batch < batches; batch += 1) {
    let a = 0;
    let b = 1;
    for (let i = 0; i < 40; i += 1) {
      const next = a + b;
      a = b;
      b = next;
    }
    checksum = b;
  }
  return checksum;
}

function workload(iterations, seed) {
  return `layout:${layoutKernel(100, 0)}`;
}
