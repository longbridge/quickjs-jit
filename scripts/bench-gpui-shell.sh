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
grep -q 'JitRuntime' "$engine" || missing+=("ShellRuntime ownership of a JIT runtime")
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
output_json=$(python3 -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$output_json")

# Pin every fresh process to one explicitly selected CPU. This removes process
# migration from paired P99 and lifecycle observations without changing either
# runtime's workload. Override only when the benchmark host reserves another
# isolated CPU for this run.
runner=(env)
if command -v taskset >/dev/null; then
  benchmark_cpu=${GPUI_SHELL_JIT_CPU:-0}
  runner=(taskset -c "$benchmark_cpu")
else
  benchmark_cpu=${GPUI_SHELL_JIT_CPU:-unavailable}
  echo "warning: taskset is unavailable; recording an unpinned benchmark" >&2
fi

cargo test --manifest-path "$shell_root/Cargo.toml" -p gpui-shell --release --offline --no-run
target_root=${CARGO_TARGET_DIR:-$shell_root/target}
test_binary=
while IFS= read -r candidate; do
  if "$candidate" --list 2>/dev/null | grep '^tests::benchmark::emit_one_jit_acceptance_sample:' >/dev/null; then
    test_binary=$candidate
    break
  fi
done < <(find "$target_root/release/deps" -maxdepth 1 -type f -perm +111 -name 'gpui_shell-*' | sort)
if [[ -z "$test_binary" ]]; then
  echo "unable to locate gpui-shell library test binary" >&2
  exit 4
fi
"${runner[@]}" "$test_binary" tests::benchmark::describing_a_panel_stays_inside_the_frame_budget --exact
"${runner[@]}" "$test_binary" tests::benchmark::numeric_layout_installs_and_enters_native_code --exact

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

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

test_binary_sha256=$(sha256 "$test_binary")
integration_patch_sha256=$(sha256 "$repo_root/integrations/gpui-component/rquickjs-jit.patch")
GPUI_SHELL_TEST_BINARY_SHA256="$test_binary_sha256" \
GPUI_SHELL_INTEGRATION_PATCH_SHA256="$integration_patch_sha256" \
GPUI_SHELL_JIT_CPU="$benchmark_cpu" \
python3 "$repo_root/benchmarks/aggregate_gpui_shell.py" \
  "$sample_dir" "$shell_root" "$repo_root" "$output_json"

cargo run --release -p rquickjs-jit-benchmarks --bin jit-gpui-shell-report -- \
  "$output_json" "${output_json%.json}.md"
