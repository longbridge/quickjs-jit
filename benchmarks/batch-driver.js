// Shared by QuickJS and Bun. Result storage consumes every call; checksum runs
// after timing. Preserve two-argument kernels' callable signature in both engines.
var benchmarkResults = [];
function benchmarkBatch(count) {
  benchmarkResults = new Array(count);
  function callWorkload() {
    return globalThis.workloadArgument === undefined
      ? workload(2000, 0) : workload(2000, 0, globalThis.workloadArgument);
  }
  function next(index) {
    for (let i = index; i < count; i++) {
      const result = callWorkload();
      if (result && typeof result.then === 'function') {
        return result.then(function(value) {
          benchmarkResults[i] = value;
          return next(i + 1);
        });
      }
      benchmarkResults[i] = result;
    }
  }
  return next(0);
}
function benchmarkChecksum() {
  return benchmarkResults.map(function(value) {
    if (typeof value === 'number') {
      const view = new DataView(new ArrayBuffer(8));
      view.setFloat64(0, value, false);
      return 'number:' + view.getBigUint64(0, false).toString(16).padStart(16, '0');
    }
    if (typeof value === 'string') return 'string:' + value;
    if (typeof value === 'boolean') return 'boolean:' + value;
    if (value === null) return 'null';
    if (value === undefined) return 'undefined';
    throw new Error('benchmark checksum primitive required');
  }).join('|');
}
