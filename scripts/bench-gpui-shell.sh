#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/bench-gpui-shell.sh GPUI_COMPONENT_ROOT OUTPUT_JSON" >&2
  exit 2
fi

shell_root=$1
output_json=$2
manifest="$shell_root/crates/shell/Cargo.toml"
engine="$shell_root/crates/shell/src/engine/quickjs/mod.rs"
benchmark="$shell_root/crates/shell/src/tests/benchmark.rs"

for file in "$manifest" "$engine" "$benchmark"; do
  if [[ ! -f "$file" ]]; then
    echo "gpui-shell integration missing required file: $file" >&2
    exit 2
  fi
done

missing=()
grep -q 'rquickjs-jit' "$manifest" || missing+=("Cargo.toml dependency rquickjs-jit")
grep -q 'quickjs-jit' "$manifest" || missing+=("native quickjs-jit feature")
grep -q 'Jit::attach' "$engine" || missing+=("ShellRuntime ownership of a JIT attachment guard")
grep -q 'GPUI_SHELL_JIT_SAMPLE' "$benchmark" || missing+=("fresh-process JSON sample emitter")
if (( ${#missing[@]} > 0 )); then
  echo "gpui-shell is not actually integrated with this JIT:" >&2
  for item in "${missing[@]}"; do
    echo "  - missing $item" >&2
  done
  echo "The in-repository gpui_shell_surface test is a fixture and cannot satisfy this gate." >&2
  exit 3
fi

repo_root=$(cd "$(dirname "$0")/.." && pwd)
shell_root=$(cd "$shell_root" && pwd)
output_json=$(realpath -m "$output_json")

# Pin every fresh process to one explicitly selected CPU. This removes process
# migration from paired P99 and lifecycle observations without changing either
# runtime's workload. Override only when the benchmark host reserves another
# isolated CPU for this run.
benchmark_cpu=${GPUI_SHELL_JIT_CPU:-0}
if ! command -v taskset >/dev/null; then
  echo "gpui-shell acceptance requires taskset for reproducible CPU affinity" >&2
  exit 5
fi
runner=(taskset -c "$benchmark_cpu")

cargo test --manifest-path "$shell_root/Cargo.toml" -p gpui-shell --release --offline --no-run
target_root=${CARGO_TARGET_DIR:-$shell_root/target}
test_binary=
while IFS= read -r candidate; do
  if "$candidate" --list 2>/dev/null | grep '^tests::benchmark::emit_one_jit_acceptance_sample:' >/dev/null; then
    test_binary=$candidate
    break
  fi
done < <(find "$target_root/release/deps" -maxdepth 1 -type f -executable -name 'gpui_shell-*' | sort)
if [[ -z "$test_binary" ]]; then
  echo "unable to locate gpui-shell library test binary" >&2
  exit 4
fi
"${runner[@]}" "$test_binary" tests::benchmark::jit_does_not_change_snapshot_or_render_count --exact

sample_dir=$(mktemp -d "${TMPDIR:-/tmp}/gpui-shell-jit-samples.XXXXXX")
trap 'rm -rf "$sample_dir"' EXIT
sample_test=tests::benchmark::emit_one_jit_acceptance_sample
for warmup in $(seq 0 4); do
  for workload in panel compute; do
    for mode in interpreter automatic; do
      GPUI_SHELL_JIT_SAMPLE="$sample_dir/warmup-$warmup-$workload-$mode.json" \
      GPUI_SHELL_JIT_MODE="$mode" GPUI_SHELL_JIT_PAIR="$warmup" \
      GPUI_SHELL_JIT_WORKLOAD="$workload" \
        "${runner[@]}" "$test_binary" "$sample_test" --exact >/dev/null
    done
  done
done
for pair in $(seq 0 29); do
  if (( pair % 2 == 0 )); then modes=(interpreter automatic); else modes=(automatic interpreter); fi
  for workload in panel compute; do
    for mode in "${modes[@]}"; do
      GPUI_SHELL_JIT_SAMPLE="$sample_dir/pair-$pair-$workload-$mode.json" \
      GPUI_SHELL_JIT_MODE="$mode" GPUI_SHELL_JIT_PAIR="$pair" \
      GPUI_SHELL_JIT_WORKLOAD="$workload" \
        "${runner[@]}" "$test_binary" "$sample_test" --exact >/dev/null
    done
  done
done

test_binary_sha256=$(sha256sum "$test_binary" | cut -d ' ' -f 1)
integration_patch_sha256=$(sha256sum "$repo_root/integrations/gpui-component/rquickjs-jit.patch" | cut -d ' ' -f 1)
GPUI_SHELL_TEST_BINARY_SHA256="$test_binary_sha256" \
GPUI_SHELL_INTEGRATION_PATCH_SHA256="$integration_patch_sha256" \
python3 "$repo_root/benchmarks/aggregate_gpui_shell.py" \
  "$sample_dir" "$shell_root" "$repo_root" "$output_json"

cargo run --release -p rquickjs-jit-benchmarks --bin jit-gpui-shell-report -- \
  "$output_json" "${output_json%.json}.md"
