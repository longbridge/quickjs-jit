#!/usr/bin/env bash
set -euo pipefail

mode="${1:-interpreter}"
output="${2:-target/jit-test262/${mode}.json}"

git -C sys/quickjs -c submodule.test262.update=checkout submodule update --init --depth 1 test262
test -d sys/quickjs/test262/test
cargo run -p rquickjs-jit --release --features compiler,test-support --bin jit-test262 -- \
  --mode "$mode" --output "$output"
