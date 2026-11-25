# AMD RDNA2 desktop GPU (gfx1030) form acceptance

This is the promotion rule for AMD RDNA2 desktop GPU (`gfx1030`) instruction
forms in [`docs/status.toml`](../status.toml). It implements the documentation
step of [#302](https://github.com/ranveersoni98/Xenora/issues/302). It does not
claim Xbox-hardware verification or commercial-game compatibility.

The **LLVM three-way decoder check** is the ignored `llvm-mc` comparison in
[`crates/decoder/tests/gfx1030.rs`](../../crates/decoder/tests/gfx1030.rs). It
compares Xenora's decoder with LLVM's gfx1030 decoder over the fixture and
probe encodings; it is encoding evidence, not GPU-semantics evidence.

## Status meanings

| Status | Meaning |
| --- | --- |
| `local known-answer test passed` | The implementation's CPU known-answer test passed locally. This is not independent acceptance. |
| `local Apple M5 readback passed; acceptance pending` | A local native Metal readback passed with validation, but the independence and evidence rules below are incomplete. |
| `fixture verified on Apple M5` | The host-qualified spelling used by three existing rows. `scripts/audit/status.py` treats it as equivalent to `fixture verified`; existing rows do not need re-acceptance solely for this wording. New entries should normally use `fixture verified` and record the host in the review log. |
| `fixture verified` | The form or documented family batch passed independent acceptance at a named commit, with CPU, LLVM and native Metal evidence recorded. |

A skipped test is not a pass. A shader compile without readback is not GPU
verification. Agreement with a spec-derived reference is not agreement with
Xbox hardware.

## Independence

> **Proposed policy, pending maintainer confirmation.** The independence rule in
> this section is the proposal from #302. Do not use it to promote forms until
> the maintainer confirms or revises it.

The person running the acceptance pass must not be the author of the pull
request that added the form being accepted. The runner may use the existing
tests and commands, but must record their own results and review the evidence.
If the maintainer runs the pass for forms they authored, the review-log entry
must say so explicitly; that exception is not silent self-certification.

The expected values used by the test must remain independent of the production
decoder, translator or Metal backend. Do not change fixtures, thresholds or test
filters to make a form pass.

## Acceptance procedure

1. **Use a clean named commit.** Start from a clean checkout of `main` at the
   commit being accepted. Record the full commit hash. Do not accept a working
   tree with uncommitted changes.
2. **Name the scope.** List each form, or a family batch with an explicit list of
   forms, covered by the pass. A family name alone is not enough.
3. **Run the standard gates.** Format, Clippy, workspace tests, fixture checks,
   the LLVM three-way decoder check and the native GPU suite must pass. Record
   exact commands, environment variables and pass/fail/skip counts.
4. **Run the form's CPU and Metal tests.** The CPU known-answer/reference tests
   and the native Metal readback must both pass. Metal API and shader validation
   must be enabled. Use a targeted test filter where useful, but keep the full
   standard-gate result for the same commit.
5. **Check negative cases.** Unsupported encodings, malformed inputs and any
   documented rejection cases for the form must still pass.
6. **Save raw logs.** Put raw output under ignored
   `artifacts/acceptance/<commit>/`. Do not commit raw logs.
7. **Write the review-log summary.** Add an entry to
   [`review-log.md`](review-log.md) with the commit, runner's GitHub login, host,
   scope, exact commands or log path, and CPU/LLVM/Metal results. A reviewer
   confirms the entry if the runner is not the maintainer.
8. **Promote only passed forms.** Change `cpu` and `gpu` in `docs/status.toml`
   only for forms covered by a passing entry. Regenerate the status tables and
   progress graphic, then run `python3 scripts/audit/status.py --check`.

## Standard gate set

Run on native Apple Silicon. The exact host and tool versions belong in the
review-log entry.

```sh
export CARGO_BUILD_JOBS=2
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
scripts/assemble-fixture.sh --check
LLVM_BIN="$(brew --prefix llvm)/bin" \
  cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored --nocapture
cargo test --locked -p xenora-semantic-review
scripts/test-gpu.sh
python3 scripts/audit/status.py --check
```

For a targeted semantic-review pass, record the filter and use the same Metal
validation environment as `scripts/test-gpu.sh`:

```sh
cargo test --locked -p xenora-semantic-review <cpu-test-filter>
MTL_DEBUG_LAYER=1 \
MTL_SHADER_VALIDATION=1 \
MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 \
  cargo test --locked -p xenora-semantic-review <native-metal-test-filter> \
  -- --ignored --exact --nocapture --test-threads=1
```

A targeted filter does not replace the standard gate set; it attributes the
result to a form or family batch.

## Review-log entry

Use one row per commit and scope. Add rows to
[`review-log.md`](review-log.md); keep raw output only in `artifacts/`.

| Date | Commit | Runner (GitHub login) | Host and tools | Forms or batch | Commands / raw log | CPU | LLVM | Metal | Review |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| YYYY-MM-DD | full SHA | @login | Mac, macOS, Xcode, Rust, LLVM | explicit form list | exact commands and `artifacts/acceptance/<commit>/` path | pass/fail/skip counts | pass/fail counts | pass/fail/skip counts; validation on | reviewer GitHub login or maintainer exception |

Do not write `pass` without counts. Do not count a skipped native test as Metal
evidence.

## Promotion and failures

After a passing entry, set the accepted rows to `fixture verified` and regenerate:

```sh
python3 scripts/audit/status.py
python3 scripts/audit/status.py --check
```

If any required check fails or skips, leave the form pending. Record the commit,
host, command and failure in the review log, and open a separate issue for the
failure. Do not weaken the test or change the expected result in the acceptance
pass.

The automated status-check enforcement proposed in #302 is a separate change.
Until it lands, reviewers must enforce the review-log requirement manually.
