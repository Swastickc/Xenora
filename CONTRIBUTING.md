# Contributing to Xenora

Xenora is an experimental Xbox Series X|S recompilation framework for Apple Silicon. Its graphics subsystem, XenoMetal, targets native Metal execution. Contributions should improve verified behavior rather than expand untested claims.

## Getting started

- Read `README.md` and, when present, `AGENTS.md`, `docs/interfaces.md`, `docs/status.toml` and the relevant architecture decisions.
- Discuss substantial changes in an issue before implementing them; small fixes do not require prior approval.
- Claim an issue before working on it by replying on it in plain words, after checking its comments for an existing claim. Never claim over someone else. A claim with no pull request after about one week may be taken over with a reply saying so. Coding agents follow the same rules; see "Claiming work before you start" in `AGENTS.md`.
- Use a native ARM64 macOS environment with Rust and the appropriate Xcode/Metal tools. LLVM AMDGPU tools are needed for ISA fixture generation; extra tooling depends on the subsystem.
- Install the native mingw-w64 cross-compiler with `brew install mingw-w64`
  before running workspace tests. The hosted PE fixture tests build their
  inputs from source and fail if the compiler is missing. The freestanding
  Gate A1 fixture uses clang and LLD. See
  [Gate A toolchain setup](docs/testing/gate-a.md#toolchain-setup) for linker fallbacks.
- Keep changes focused and preserve existing public interfaces unless a change is justified and documented.

## Development and verification

When the Cargo workspace exists, run from its root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

When troubleshooting more than one failure, use
`cargo test --locked --workspace --no-fail-fast` to let Cargo continue
running the remaining test targets.

The gfx1030 LLVM cross-checks are ignored in normal runs. Run them with Homebrew LLVM:

```sh
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test --locked -p xenora-decoder --test gfx1030 -- --ignored --nocapture
```

Follow repository documentation for actual GPU tests, fuzzing, mutation testing and benchmarks. Report commands, versions, pass/fail/skip counts and hardware used. A successful shader compile is not successful GPU execution. Do not report unavailable tools or skipped tests as passes.

For an instruction or compiler change, cite the applicable ISA section, state the exact architecture, document operand and execution-mask semantics, and add tests for encodings, edge cases, unsupported cases and independent expected results. Where applicable, verify the resulting MSL through actual Metal execution. Do not convert unsupported instructions into silent no-ops.

For a performance change, provide reproducible before-and-after results using equivalent inputs, hardware and settings. For a new dependency, explain why it is needed, verify its license and native ARM64 compatibility, and record its provenance.

## macOS contributor previews

Use native Apple Silicon, Python 3.12 or later and the toolchain required by
`scripts/build-app.sh`. From the repository root, commit any tracked changes
before building. The preview builds a separate snapshot of HEAD. Ignored and
untracked files are excluded from that snapshot.

```sh
mkdir -p artifacts
PYTHONPATH=scripts python3 -m preview build --build-number 42 --output artifacts/preview-42
PYTHONPATH=scripts python3 -m preview verify artifacts/preview-42
```

Choose a new output directory under an existing parent. Set the bundle build
number to an incrementing positive integer of up to four digits, optionally
followed by one or two numeric components of up to two digits each. Omit
leading zeroes. The three-component marketing version comes from
`workspace.package.version` in `Cargo.toml`. The app and manifest record the
exact Git commit.

The build runs the locked native app build and smoke checks. After signing and
archiving, it extracts the app, records strict signature, ARM64 and system
library checks, and runs the original trusted fixture through the app and its
worker with Metal validation enabled. A successful build writes `Xenora.zip`,
`manifest.json`, preview notes and license notices. The verifier compares
checksums, the signed bundle metadata and the required signatures without
launching the app. The build and fixture results are historical records.
Keep the manifest with the archive and obtain both from a trusted source.
Checksums identify changes relative to that manifest.

The default build uses local ad-hoc signatures. For an authorized Apple
submission, provide both `--developer-id` with the Developer ID Application
identity and `--notary-profile` with its existing keychain profile. Those flags
sign the app and its worker with Hardened Runtime, run the original fixture
before submission, and send a temporary ZIP to Apple. An accepted submission
is stapled to the app and verified again after the final ZIP is extracted.
Use `verify --require-developer-id <directory>` to require the Developer ID
workflow when verifying a preview. The check accepts a valid Apple Developer ID
signature and does not identify the repository's publisher. Keychain credentials
stay outside the repository and the output. Before distribution, review Apple's
warnings for the submission ID in `manifest.json` with
`xcrun notarytool log <submission-id> --keychain-profile <profile>`.

The preview tool leaves release creation and public distribution to the
maintainers. See the [remaining attribution and corresponding-source
review](docs/licenses.md#remaining-release-question) and [the public preview
tracking issue](https://github.com/ranveersoni98/Xenora/issues/415).

Run the preview unit tests with
`PYTHONPATH=scripts python3 -m unittest discover -s tests/preview`.

## Research and clean-room policy

Use publicly accessible specifications and authorized materials only. Do not submit leaked SDKs, confidential documentation, firmware, keys, proprietary game binaries or unauthorized shader dumps. Under Xenora's clean-room policy, anyone who has accessed leaked or confidential Xbox materials must not contribute. Do not post details of restricted material in issues or PRs.

Distinguish desktop RDNA behavior from Xbox-specific behavior. Cite primary sources with exact applicable architectures and record uncertainties using the project's evidence classification when available. Tests on Apple GPUs do not establish verification on Xbox hardware.

Xenora implements its own Rust guest runtime, decoder, typed IR, shader
compiler and native Metal backend from public technical specifications
and independently derived tests. Contributions must not copy, port or
adapt code, decoder tables, internal metadata or implementation
architecture from other emulators. External results can inform test
selection. Author the resulting code and fixtures independently.

Before incorporating third-party code, verify per-file license compatibility with GPL-3.0-or-later, record the exact upstream revision and original paths, preserve copyright notices and document changes. Update applicable attribution records. Public visibility alone is not proof of authorization or license compatibility.

## Pull requests

- Branch from the current main branch, keep commits focused, and link relevant issues.
- Fill in every section of the pull request template, keep `Closes #<number>` (or write `No linked issue: <reason>`), and tick the claimed-first and DCO checkboxes. The *PR template* check fails until the description does, and runs again each time you edit it.
- Include a clear explanation, tests and any documentation/status updates. Separate unverified work from supported behavior.
- Disclose material AI assistance and personally review generated changes. AI output is not an authoritative source, test oracle or substitute for independent review.
- Never include secrets, personal paths, credentials or private data in patches or logs.
- Do not claim approval from an independent reviewer unless one has actually reviewed the change.

Sign off each commit under the [Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s -m "Describe the change"
```

Use your configured Git identity and a privacy-preserving email address if desired. Sign-off is your certification of the contribution; do not sign off work on another person's behalf. Contributions are subject to the repository license and any applicable, compatible third-party notices.

For security-sensitive findings, see [SECURITY.md](SECURITY.md). Community expectations are in [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
