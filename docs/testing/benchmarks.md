# Benchmark protocol

The benchmark harness measures each fixture in its manifest that passes its
correctness test in the same run. It never reports a number for a shader that
failed or was skipped.

## Corpus

`benchmarks/manifest.toml` lists each fixture with its stage, source, binary path,
declared input and hand-derived expected output, and the word counts to time. The
manifest uses a documented subset of TOML (`[[fixture]]` tables with string and
integer-array values) parsed by the harness, so no external TOML dependency is
pulled into the workspace.

A fixture is added to the corpus by adding a table. If it passes correctness it is
benchmarked automatically; there is no separate opt-in list.

## Correctness gate

Before any timing, the harness requires all of the following for each fixture:

1. The Rust pipeline (`xenora-shader::compile`) accepts the binary.
2. The independent reference interpreter agrees with the hand-derived `expected`
   values on the declared input.
3. The native GPU produces the expected output for the smallest declared size.

If any step fails the fixture is marked `excluded` with a reason, listed in the
coverage table, and not timed. A nonempty exclusion set makes the harness exit
nonzero. Measurements are included only after all sizes for a fixture
pass. A debug smoke run using a temporary manifest with sizes 4 and
4097 for the add fixture verified that rejection at 4097 produces an
empty timing list for that fixture while the other two fixtures retain
all five sizes. The smoke run is a correctness check for reporting.

## What is measured

Per fixture and per word count, over `--reps` measured repetitions after two
warm-up repetitions:

| Field | Source | Meaning |
| --- | --- | --- |
| `compile` | host monotonic clock | Rust decode, lift, verify and MSL generation |
| `library` | host monotonic clock | `newLibraryWithSource` Metal compilation |
| `pipeline` | host monotonic clock | compute pipeline state creation |
| `gpu` | `GPUEndTime - GPUStartTime` | command-buffer GPU execution time |
| `host` | host monotonic clock | wall time of one compile-and-dispatch call |

`gpu_zero_fraction` records the share of samples whose GPU timestamp delta was
zero, which happens for very short kernels. A high value means the GPU timing is
at timer resolution and should not be read as a precise duration.

Statistics reported are median, minimum, maximum and the first and third quartiles.
The report never uses best-of.

## Protocol

- Release builds only for published numbers; the metadata records the profile.
- The machine must be otherwise idle, on AC power, with the thermal state noted.
- Validation is off for timing runs. The metadata records the `MTL_DEBUG_LAYER`
  and `MTL_SHADER_VALIDATION` values in effect so a validated run is never confused
  with a timing run.
- The fanless M5 MacBook Air throttles under sustained load; a single run does not
  characterize sustained performance.
- No comparison to Xbox hardware, other emulators or other platforms is made.

## Output

`benchmarks/results/<hardware>/<run-id>/` contains `metadata.json`, `results.json`
and `report.md`. Only small text reports are committed. Raw logs stay under the
ignored `artifacts/` directory.

## Reproduction

```sh
cargo build --release -p xenora-benchmarks
MTL_DEBUG_LAYER=0 MTL_SHADER_VALIDATION=0 ./target/release/xenora-bench --reps 7
```

## Limitations

The corpus currently covers three synthetic gfx1030 compute fixtures under the
[synthetic memory ABI](../adr/0003-demo-memory-model.md). It does not measure
graphics stages, guest containers, Wave64 or divergence. Translation-versus-native
baseline ratios are not reported because no hand-written native MSL baseline exists
yet for these fixtures.

## Exploratory AC run

Run `1795352394` used the release harness built at `cc0daa7` on Apple M5,
with AC power confirmed before and after, 30 seconds between the build and
timing, two warmups, 21 measured repetitions per size and Metal validation
off. Neither thermal nor performance warnings were recorded by
`pmset -g therm`. All three declared translated fixtures passed their
correctness checks. The recorded 1-, 5- and 15-minute load averages were
8.12, 8.68 and 6.31, so an idle environment was not established. G10 remains open for the full correctness corpus,
individual compiler stages and cold/warm timing, tool and capability metadata,
and noise-based variability reporting. See the
[generated report](../../benchmarks/results/apple-m5/run-1795352394/report.md).

## Guest service dispatch

`cargo bench --locked -p xenora-loader --bench service_dispatch` measures
original guest loops that call standard runtime services through a bound
import table under the ARM64 dynamic binary translator. Before timing, each
workload checks its result: the loop count reaches zero, the service returns
its documented value, and `qsort` leaves the array in the order Rust's own sort
produces. The empty loop is reported separately, so readers can subtract the
loop and translation overhead. Each figure is the median per-iteration time of
10 measured batches of 10,000 iterations, after one warm-up batch.

Measured on Apple M2 Pro (12 logical processors), release profile, AC power,
on the working tree after `ed9b93c`. The 1-minute load average was 7.82, so the
machine was not idle and results are indicative only.

| Workload | Before (ns) | After the block `strlen` scan (ns) |
| --- | --- | --- |
| Empty loop | 21.5 | 20.1 |
| `GetLastError` | 181.8 | 188.4 |
| `WaitForSingleObject`, signaled manual-reset event | 281.0 | 284.6 |
| `strlen`, 63 bytes | 1,990.4 | 223.1 |
| `HeapAlloc` + `HeapFree`, 64 bytes | 475.0 | 493.3 |
| `qsort`, 16 ints with a guest comparator | 10,242.0 | 11,396.7 |

The `strlen` change scans 256-unit blocks with one checked read each. It falls
back to per-unit reads only when a block crosses unmapped or unreadable
memory, so the existing exact-fault tests still pass. The other rows differ by up to 11 percent between runs on this loaded
machine; that is run-to-run variation, not a measured change. Raw logs are kept locally under
`artifacts/system-runtime/bench/`. No comparison with native Windows is made.
