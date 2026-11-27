# Input, host integration and playability campaign (Campaign F)

Tracking issue: [#942](https://github.com/ranveersoni98/Xenora/issues/942). Owner: @Swastickc.

This campaign makes the native macOS app usable as an interactive environment for original guest programs. Host keyboard, mouse and physical controllers reach the guest through the guest's own XInput calls. They travel over the XInput snapshot and FFI path that already exists:

- the controller-state boundary (#847);
- snapshot serialization (#849);
- the host controller bridge (#857);
- capabilities (#859);
- the extended surfaces (#872);
- slot mapping (#874);
- vibration (#875);
- battery evidence (#914).

The campaign adds no second guest controller, and it does not rewrite that backend.

Commercial game execution is not supported. Nothing in this document claims it.

## Status

| Task | What | Status | Evidence |
| --- | --- | --- | --- |
| F1 | Keyboard to virtual Xbox controller | Partial: host-agnostic translator only | `crates/input/tests/keyboard.rs` (#944) |
| F2 | Mouse and trackpad to the right stick | Partial: host-agnostic translator only | `crates/input/tests/mouse.rs` (#944) |
| F3 | Host input router | Partial: routing model and session C interface; the app does not use it yet | `crates/input/tests/router.rs`, `review.rs` (#944); `crates/ffi/tests/input_router*.rs`, `input_recognize.rs` (#953) |
| F4 | Versioned mapping profiles | Partial: profiles and text format, not wired to the app | `crates/input/tests/profile_format.rs` (#948) |
| F5 | SwiftUI input settings | Not started | |
| F6 | Per-application configuration | Not started | |
| F7 | Window and presentation | Not started | |
| F8 | Session lifecycle and pause | Not started | |
| F9 | Bounded host event delivery | Not started | |
| F10 | Input recording and replay | Not started (coordinate with #154) | |
| F11 | Diagnostics HUD | Not started | |
| F12 | Headless interactive testing | Partial: one scripted, deterministic guest case | `crates/input/tests/guest_xinput.rs` (#951) |
| F13 | Accessibility of the input UI | Not started (coordinate with #409) | |
| F14 | Original interactive guest | Not started | |
| F15 | Sustained reliability | Not started | |
| F16 | This document | Active | |
| F17 | Physical controller support | Partial: recognition, translation and guest delivery with virtual devices; not yet wired to the app | `crates/input/tests/controller.rs`, `review_profiles.rs` (#948), `guest_xinput.rs` (#951) |

"Partial" means the code exists and its tests pass, but a guest cannot use it yet.

## Planned pull requests, in dependency order

1. **Host input router (#944).** The `crates/input` crate, plus this document.
2. **Physical controller profiles and translation (F17, F4).** Controller family recognition, the Xbox logical layout for PlayStation and Nintendo controllers, radial dead zones, trigger thresholds, response curves, drift calibration, and a versioned, validated profile format.
3. **Router in the session and the app.** FFI functions feed the router. The app delivers key and mouse events from the focused game view only, with no global monitoring, and neutralizes input when focus is lost. GameController values go into the router. Router output fills the existing XInput slots.
4. **Recording, replay and headless runs (F10, F12).** A versioned, bounded input recording format replays into `xenora run`. An original C PE32+ guest reads XInput through the ARM64 DBT, which is the F17 acceptance with virtual devices.
5. **Live input for chosen programs.** A bounded input channel from the app into the `xenora run` worker, so a program the user runs receives physical controller input.
6. **Input settings and accessibility (F5, F13).**
7. **Per-application configuration (F6).**
8. **Lifecycle, pause and host events (F8, F9), then window and presentation (F7).**
9. **Diagnostics HUD (F11).**
10. **Original interactive guest (F14).**
11. **Reliability runs (F15).**

## The host input router (`crates/input`)

The crate is pure Rust with no dependencies. It is deterministic: the caller supplies time in milliseconds, so the same event script always produces the same output. It reads no input devices itself, and it talks to no guest.

It produces up to four per-slot snapshots in public XInput units:

- the button bits;
- triggers from 0 to 255;
- thumb axes from -32768 to 32767, with Y positive upward.

### Keyboard (F1)

Bindings name physical macOS virtual key codes (the `kVK_*` values in Carbon `Events.h`). They do not depend on the keyboard layout, the character a key produces, or modifier state. The default bindings are:

| macOS key | Guest input |
| --- | --- |
| W, A, S, D | Left stick |
| Return, Space | A |
| Escape, Backspace (Delete) | B |
| X, Y | X, Y |
| Arrow keys | Directional pad |
| Q, E | Left and right shoulder |
| Z, C | Left and right trigger |
| Tab | Menu (`START`) |
| V | View (`BACK`) |
| Left Shift, Right Shift | Left and right stick click |

How keys combine:

- Keys can be held together.
- Opposite directions cancel.
- Diagonals are normalized to unit length.
- An optional linear ramp, capped at ten seconds, turns a held key into a growing stick deflection. A repeated key-down does not restart the ramp.
- Buttons and triggers never ramp.
- A scale factor limits keyboard stick deflection.
- Releasing everything at once (used on focus loss) leaves nothing stuck.

The configuration names exactly one public XInput bit per button binding. Out-of-range values are refused. Keys bound to more than one different target are reported, so the settings UI can warn about them.

### Mouse and trackpad (F2)

Relative motion deflects the right stick. The settings are:

- per-axis sensitivity, as deflection per point of motion;
- inversion;
- a radial dead zone;
- exponential smoothing;
- a maximum magnitude;
- a decay interval after which the stick returns to neutral when motion stops.

Mouse buttons map to buttons or triggers. By default, the primary button is the right trigger and the secondary button is the left trigger. These defaults are a Xenora choice.

Deflection is computed per motion sample. A host that delivers many small events per frame should add up the deltas for each display frame before passing them in, so that deflection does not depend on the mouse's event rate. The app wiring will do that.

Pointer capture is not part of this crate. It belongs to the app wiring, and it must release on focus loss.

### Routing (F3)

The sources are the virtual keyboard-and-mouse pad, which is always connected and needs no hardware, and up to four physical devices, identified by stable host IDs. A device takes the lowest free slot when it connects and keeps that slot until it disconnects. A device that reconnects, or that replaces another on the same slot, starts neutral.

| Mode | Slot behavior |
| --- | --- |
| Automatic (default) | Devices on their own slots; the keyboard and mouse merge into slot 0. |
| Keyboard and mouse only | Slot 0 is the keyboard and mouse; devices are tracked but ignored. |
| Physical only | Devices on their own slots; the keyboard and mouse are ignored. |
| Combined | Every source merges into slot 0. |
| Last active | Slot 0 follows the source that most recently produced new non-neutral input. |
| Explicit | The sources assigned to each slot merge there; unassigned sources are ignored. |

How sources merge:

- Buttons are combined with OR, so one source never releases a button that another still holds.
- Each trigger takes the maximum value.
- Each stick takes the whole axis pair from the source with the largest deflection. A tie goes to the earlier source, so the result is deterministic.

At most sixteen sources can hold an explicit assignment.

When the window loses focus, the router neutralizes every source and refuses input until focus returns.

These routing rules are Xenora host policy. They are not Windows or Xbox assignment behavior.

## Physical controller profiles (F17, F4)

`xenora_input::controller` names the GameController elements by position. The `GCExtendedGamepad` header documents the face buttons as A at the bottom, B on the right, X on the left and Y at the top. So every controller family, PlayStation and Nintendo included, reaches the guest in the Xbox logical layout, with no keyboard emulation.

Family recognition matches the documented product category strings exactly:

| Category | Extra evidence | Family |
| --- | --- | --- |
| `Xbox One` (`GCProductCategoryXboxOne`) | paddle buttons present (`GCXboxGamepad.h`: Elite) | Xbox Elite |
| `Xbox One` | Share button present (`GCXboxGamepad.h`: Series X\|S) | Xbox Series X\|S |
| `Xbox One` | neither | Xbox One |
| `DualSense` (`GCProductCategoryDualSense`) | | DualSense |
| `DualShock 4` (`GCProductCategoryDualShock4`) | | DualShock 4 |
| anything else, extended gamepad | | Generic extended |
| no extended gamepad | | Micro |

There is no public category for the Switch Pro controller or the DualSense Edge. Neither is recognized automatically. A Switch Pro profile can be chosen explicitly, in positional form or with the printed-label layout, which swaps A with B and X with Y. A DualSense Edge is expected to report as a DualSense, but that is untested.

A profile can change:

- what each element drives: a button, a trigger or a stick direction;
- each stick's radial dead zone, outer edge, response curve (linear, quadratic or cubic), drift calibration offset, inversion and sensitivity;
- whether the sticks are swapped;
- each trigger's threshold, and whether the trigger acts digitally;
- vibration intensity.

Dead zones default to zero, because XInput applications apply their own. The default profile is a bit-exact per-axis passthrough of the host values, the same as the existing bridge, and `review_profiles.rs` checks that over a sweep of 32-bit values. Profiles are saved in a versioned line-oriented text format, limited to 16 KiB, 256 lines and 64 bindings. Malformed text is refused with a reason. A newer version is refused and named in the message, and a valid profile round-trips exactly.

## A guest reads routed input through XInput (F17, F12)

`fixtures/pe/xinput-poll.c` is an original freestanding C program, built by `fixtures/pe/build-xinput-poll.sh` with native ARM64 clang and lld, and the build is reproducible. Each frame it calls Xenora's frame-tick service, `xenora_session.dll!XenoraNextEvent`, the same one the persistent guest uses. It then reads user slots 0 to 3 through its own `xinput1_4.dll!XInputGetState` calls and prints a line for every packet change, connection and disconnection.

`crates/input/tests/guest_xinput.rs` runs the guest through the loader and the ARM64 DBT, and checks that translated blocks and service calls occurred. On each tick it applies one frame of scripted host events to the router, then installs the router output through the public `ServicesState::set_xinput_controller`. The script covers:

- keyboard keys;
- mouse motion;
- two virtual DualSense-profile controllers on slots 0 and 1;
- a button held by one source while another releases it;
- focus loss;
- a disconnect.

The guest's output is compared with a hand-written literal. The same script in physical-only mode produces a different hand-written literal, and repeated runs are identical. Nothing reaches the guest except its own XInput calls.

These are virtual devices: the raw element values stand in for what GameController reports. The app does not route live input this way yet.

## The session C interface

Each FFI session owns one router. The `xenora_session_input_*` functions feed it, and `xenora_session_input_snapshot` reads it back, for a settings screen. `xenora_session_input_publish` installs its output into the running frame loop's existing XInput slots. `xenora_controller_recognize` exposes the family rule. See [the interface description](../interfaces.md#host-input-router). The CPU tests drive the interface without a GPU. A GPU-gated test publishes into a running native frame loop and checks connection changes through the existing vibration poll.

## Hardware evidence

No physical controller has been tested in this campaign so far. None was connected to the development Mac: GameController reported zero controllers. Every controller family named in the directive is untested here: Xbox Series X|S, Xbox One, Elite Series 2, DualShock 4, DualSense, DualSense Edge, Switch Pro, and third-party USB or Bluetooth controllers.

## Acceptance commands

```sh
cargo test -p xenora-input --locked
fixtures/pe/build-xinput-poll.sh   # rebuilds fixtures/pe/xinput-poll.exe byte for byte
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```
