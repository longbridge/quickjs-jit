function workload(iterations, seed) {
  const pieces = [];
  for (let i = 0; i < iterations; i++) pieces.push("item-" + ((i + seed) % 97) + ":" + (i % 13));
  const text = pieces.join("|");
  const matches = text.match(/item-(?:1[0-9]|2[0-9]|3[0-2]):\d+/g) || [];
  const replaced = text.replace(/item-(\d+):(\d+)/g, "$2@$1");
  return matches.length + ":" + replaced.length + ":" + replaced.slice(-16);
}
