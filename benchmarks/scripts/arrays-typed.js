function getElement(values, index) {
  let value = values[index];
  return value + 0;
}

function setElement(values, index, value) {
  values[index] = value;
  return value + 0;
}

function workload(iterations, seed) {
  const values = [];
  for (let i = 0; i < iterations; i++) values.push((i * 17 + seed) & 0xffff);
  let sum = 0;
  for (let i = 0; i < values.length; i++) sum = (sum + getElement(values, i)) | 0;
  const ints = new Int32Array(values);
  const floats = new Float64Array(iterations);
  let mixed = 0.0;
  for (let i = 0; i < iterations; i++) {
    setElement(floats, i, getElement(ints, i) * 0.25 + 0.5);
    mixed += getElement(floats, i);
  }
  return sum + ":" + mixed.toFixed(3) + ":" + values.length;
}
