function workload(iterations) {
  let text = "";
  for (let i = 0; i < iterations; i++) text += String.fromCharCode(97 + (i % 26));
  return JSON.stringify({length: text.length, first: text.slice(0, 8)});
}
