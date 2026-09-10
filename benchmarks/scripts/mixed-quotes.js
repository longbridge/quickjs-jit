// Pure JS adaptation of gpui-kit shell MIXED_MARKET_TEMPLATE. Each workload
// is one 96-quote render kernel; no GPUI snapshot building or host validation.
function quoteScore(seed, index) {
  let previous = seed;
  let current = seed + index + 1;
  let aggregate = 0;
  for (let sample = 0; sample < 32; sample += 1) {
    const next = previous + current;
    previous = current;
    current = next;
    aggregate += current & 2047;
  }
  return aggregate;
}
function compareQuotes(left, right) {
  return right.score - left.score;
}
function workload(iterations, seed) {
  const quotes = [];
  let total = 0;
  for (let index = 0; index < 96; index += 1) {
    const score = quoteScore(17, index);
    total += score;
    quotes.push({ index, score });
  }
  quotes.sort(compareQuotes);
  let visible = '';
  for (let rank = 0; rank < 12; rank += 1) {
    const quote = quotes[rank];
    visible += 'SYM' + quote.index + ':' + quote.score + ';';
  }
  return 'total:' + total + ';' + visible;
}
