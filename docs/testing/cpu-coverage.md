# CPU test runtime and shard selection

This page records what the CPU test gate costs and describes
`scripts/cpu-tests.sh`, which splits that gate into deterministic shards. The
shards are a convenience for local iteration and parallel runners. They do not
replace any gate: `cargo test --locked --workspace` stays the full CPU gate, and
`scripts/test-gpu.sh` stays the GPU gate.

## What counts as a CPU test

A CPU test is any test target that `cargo test --locked --workspace` builds with
default features. Targets that need a Metal device are either behind the
`gpu-tests` feature (`xenora-metal`, `xenora-loader`, `xenora-ffi`) or marked
`#[ignore = "requires a native Metal device ..."]` (for example
`tests/validation/known_answers.rs`), so the default run does not execute them.
`scripts/test-gpu.sh` runs them. The selection uses Cargo's own target list, so
it never decides by file name which tests are CPU tests.

One exception: two `xenora-ffi` library tests
(`test_resource_management_lifecycle_and_validation` and
`test_deferred_resource_retirement_waits_for_in_flight`) call
`xenora_session_create_texture`. When a Metal device is available, that
function creates one and uploads a texture; when none is available, it
continues without one. So on a Mac these default tests do touch Metal, and they
are not GPU verification. The selection keeps them because the full gate runs
them.

## Usage

```sh
scripts/cpu-tests.sh --list         # every default-feature test target, sorted
scripts/cpu-tests.sh --shard 2/3    # run shard 2 of 3
scripts/cpu-tests.sh --check 3      # show that 3 shards cover every target once
```

Targets are sorted by package, kind and name. Shard K of N takes each target
whose zero-based index i satisfies i mod N = K - 1. Shard 1 also runs the
workspace doctests. A new target joins a shard without any edit to the script.
The script exits 1 when any selected target fails or when Cargo cannot build the
workspace. Partial target lists never run. It exits 2 on bad arguments.
It uses at most four Cargo build jobs unless `CARGO_BUILD_JOBS` says otherwise.

## Measurements

Host: Apple M4 (10 cores), macOS on ARM64, `cargo 1.100.0`, target
`aarch64-apple-darwin`, debug test profile, main at `d7f7d58`. All runs used a
warm build unless noted. These are single-host samples, not benchmarks.

Full gate, `cargo test --locked --workspace`, three runs: 40.47 s, 39.99 s and
40.12 s wall time. It ran 82 test targets plus doctests (92 `test result` lines):
681 passed, 0 failed, 34 ignored.

Running each target alone through `cargo test -p <package> <selector>` adds up
to 79.5 s, because every invocation pays Cargo start-up. The five most expensive
targets, one sample each:

| Target | Wall time | Passed / ignored |
| --- | --- | --- |
| `xenora-loader --test runtime_services` | 19.13 s | 23 / 0 |
| `xenora-loader --test cpu_known_answers` | 8.18 s | 42 / 5 |
| `xenora-loader --test probe_delay_imports` | 3.53 s | 8 / 0 |
| `xenora-loader --test probe_truncation` | 3.32 s | 1 / 0 |
| `xenora-ffi --lib` | 2.16 s | 18 / 0 |

`runtime_services` alone is about a quarter of the summed cost. Any shard that
contains it takes at least about 19 s. The next step for a faster gate is
profiling that target, not finer sharding.

Shard runs, one sample each, sequential on the same host:

| Selection | Wall time per shard | Passed | Failed | Ignored | Result lines |
| --- | --- | --- | --- | --- | --- |
| 2 shards | 37.98 s, 4.55 s | 360 + 321 = 681 | 0 | 21 + 13 = 34 | 51 + 41 = 92 |
| 3 shards | 6.54 s, 29.55 s, 6.27 s | 160 + 282 + 239 = 681 | 0 | 9 + 12 + 13 = 34 | 38 + 27 + 27 = 92 |

For both shard counts, the union of the shards matches the full gate exactly
in passed, failed, ignored and result-line counts. Round-robin by name does
not balance cost: with one dominant target, parallel shards finish no sooner
than the shard that holds it. Running the shards one after another costs more
than the full gate, because each target pays Cargo start-up.

## Verification of the script

- `--check N` for N = 1 to 4 reports that all 82 targets are covered once.
- `--check 3` fails when the selection expression is changed to skip a
  residue or to repeat a target.
- `--list` exits 1 with an error when a crate fails to compile, instead of
  printing a shortened list.
- Bad arguments (`--shard 0/2`, `--shard 3/2`, `--check x`) exit 2.

## Limitations

- Timings come from one machine and few samples. Re-measure before using them
  for decisions.
- Shard membership changes when targets are added or renamed. Shards from two
  different commits are not comparable.
- The experiment workspace `experiments/x1` is outside the selection. CI runs
  it separately.
