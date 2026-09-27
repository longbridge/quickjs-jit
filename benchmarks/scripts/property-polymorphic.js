// Focused polymorphic-property kernel: the `point.x`/`point.y` sites each
// observe four stable own-data shapes, one of them prototype-less. The
// receivers are a benchmark argument so allocation stays outside the measured
// function; selecting them by branch (not by array element) keeps the hot
// sites on the optimized tier's bounded polymorphic inline cache.
globalThis.workloadArgument = {
  a: { x: 1, y: 10 },
  b: { y: 20, x: 2, z: 5 },
  c: { tag: 7, x: 3, y: 30 },
  d: Object.assign(Object.create(null), { y: 40, w: 9, x: 4 }),
};

function workload(iterations, seed, shapes) {
  let sum = seed;
  let point = shapes.a;
  for (let i = 0; i < iterations; i++) {
    const k = i & 3;
    if (k === 0) point = shapes.a;
    else if (k === 1) point = shapes.b;
    else if (k === 2) point = shapes.c;
    else point = shapes.d;
    sum = sum + point.x + point.y;
  }
  return sum;
}
