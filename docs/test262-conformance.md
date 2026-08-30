# Test262 conformance report

Pinned Test262 revision: `d5e73fc8d2c663554fb72e2380a8c2bc1a318a33`

The applicable-corpus gate evaluates every discovered test after applying the
checked, accountable exclusions in
`jit/tests/fixtures/test262-exclusions.json`, the pinned QuickJS
`test262.conf`, and exact paths in the pinned `test262_errors.txt` baseline.

## 2026-08-30 full run

| Mode | Files discovered | Variants | Passed | Accountable skips | Failed |
| --- | ---: | ---: | ---: | ---: | ---: |
| Interpreter | 53,169 | 102,117 | 80,371 | 21,746 | 0 |
| Automatic JIT | 53,169 | 102,117 | 80,371 | 21,746 | 0 |

The sorted `(path, variant, status, skip_reason)` tuples were identical between
the two modes. Machine-readable reports are generated at
`target/jit-test262/interpreter-full-final.json` and
`target/jit-test262/automatic-full-final.json`.

Run either complete mode with:

```sh
scripts/run-jit-test262.sh interpreter target/jit-test262/interpreter-full-final.json
scripts/run-jit-test262.sh automatic target/jit-test262/automatic-full-final.json
```

Compare the complete result sets with:

```sh
jq -s '((.[0].cases | map({path,variant,status,skip_reason}) | sort_by(.path,.variant)) == (.[1].cases | map({path,variant,status,skip_reason}) | sort_by(.path,.variant)))' target/jit-test262/interpreter-full-final.json target/jit-test262/automatic-full-final.json
```
