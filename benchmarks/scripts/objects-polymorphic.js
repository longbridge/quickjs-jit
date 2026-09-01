function workload(iterations, seed) {
  const objects = [];
  for (let i = 0; i < iterations; i++) {
    let object;
    switch (i & 3) {
      case 0: object = { x: i, y: seed, kind: 0 }; break;
      case 1: object = { y: seed, x: i, extra: 1, kind: 1 }; break;
      case 2: object = { x: i, kind: 2 }; object.y = seed; break;
      default: object = Object.create(null); object.kind = 3; object.x = i; object.y = seed;
    }
    object.x += object.kind;
    objects.push(object);
  }
  let sum = 0;
  for (let i = 0; i < objects.length; i++) sum += objects[i].x + objects[i].y;
  return sum;
}
