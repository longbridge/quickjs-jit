/* Adapted from sys/quickjs/tests/microbench.js (MIT), int_arith.
 * Copyright (c) 2017-2019 Fabrice Bellard and Charlie Gordon.
 */
function workload(iterations) {
  let globalResult = 0;
  for (let j = 0; j < iterations; j++) {
    let sum = 0;
    for (let i = 0; i < 1000; i++) sum += i * i;
    globalResult += sum;
  }
  return String(globalResult);
}
