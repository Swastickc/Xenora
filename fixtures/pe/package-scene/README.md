# Packaged scene fixture

This original PE32+ program loads `xenora_scene.dll` through Xenora's
explicit package runner. The provider checks that its initialized pointer
was relocated and makes its scene geometry available after process attach.
A second provider with the same basename rejects process attach.

The program draws two 4×3 frames with the reference renderer. It checks
each RGBA8 pixel, consumes an auto-reset event per frame, and writes both
readbacks to one guest file. The first frame is opaque red. The second
triangle lies outside the viewport and leaves the opaque clear color
`(10, 20, 30, 255)`. The program closes its file and event and releases
its graphics resources before exit.

Run from the repository root.

```sh
fixture=fixtures/pe/package-scene
entry="$fixture/xenora-package-scene.exe"
digest=$(shasum -a 256 "$entry" | cut -d ' ' -f 1)
cargo run --locked -p xenora --bin xenora -- run --experimental \
  --pe "$entry" --package "$fixture/xenora_scene.dll" --sha256 "$digest" \
  --export-guest-file 'C:\package-frames.bin' packaged-frames.rgba --trace
```

Use a new export path. The file contains 96 bytes, with the two frames
stored consecutively in row order. The SHA-256 option verifies the entry
executable. The DLL path is supplied explicitly. The CLI tests check
repeated exports, digest refusal, a missing provider and attach refusal.

```sh
fixtures/pe/package-scene/build-package-scene.sh --check
cargo test --locked -p xenora --test packaged_scene_cli
```

The first command rebuilds the three images with native ARM64 Clang and
LLD and compares them to the checked-in fixtures. Run the build script
without `--check` to update the images after changing the fixture sources.
