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
| F3 | Host input router | Partial: routing model only, not wired to the app | `crates/input/tests/router.rs`, `review.rs` (#944) |
| F4 | Versioned mapping profiles | Not started | |
| F5 | SwiftUI input settings | Not started | |
| F6 | Per-application configuration | Not started | |
| F7 | Window and presentation | Not started | |
| F8 | Session lifecycle and pause | Not started | |
| F9 | Bounded host event delivery | Not started | |
| F10 | Input recording and replay | Not started (coordinate with #154) | |
| F11 | Diagnostics HUD | Not started | |
| F12 | Headless interactive testing | Not started (coordinate with #793) | |
| F13 | Accessibility of the input UI | Not started (coordinate with #409) | |
| F14 | Original interactive guest | Not started | |
| F15 | Sustained reliability | Not started | |
| F16 | This document | Active | |
| F17 | Physical controller support | Existing bridge only (#857, #874, #875, #914) | |

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

## Hardware evidence

No physical controller has been tested in this campaign so far. Every controller family named in the directive is untested here: Xbox Series X|S, Xbox One, Elite Series 2, DualShock 4, DualSense, DualSense Edge, Switch Pro, and third-party USB or Bluetooth controllers.

## Acceptance commands

```sh
cargo test -p xenora-input --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```
