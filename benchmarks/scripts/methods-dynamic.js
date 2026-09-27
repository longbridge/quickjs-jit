function Vec(x, y) { this.x = x; this.y = y; }
Vec.prototype.dot = function (other) { return this.x * other.x + this.y * other.y; };
Vec.prototype.scale = function (k) { return new Vec(this.x * k, this.y * k); };
function describe(value) {
  if (typeof value === 'undefined') return 0;
  if (typeof value === 'function') return 1;
  if (value instanceof Vec) return 2;
  if (typeof value !== 'object') return 3;
  if ('kind' in value) return 4 + value.kind;
  return 7;
}
function tally(counts, key) { counts[key] += 1; }
function workload(iterations, seed) {
  const values = [undefined, describe, new Vec(1, 2), { kind: 1 }, { other: 1 }, 'text', { kind: 2 }];
  const counts = [0, 0, 0, 0, 0, 0, 0, 0];
  let sum = 0;
  for (let i = 0; i < iterations; i++) {
    const v = new Vec(i & 7, seed + 1);
    const { x, y } = v.scale(2);
    sum += v.dot({ x, y }) + (x ** 2) % 5;
    tally(counts, describe(values[i % values.length]));
  }
  return sum + ':' + counts.join(',');
}
