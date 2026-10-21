# Contributing

Bug reports, fixes, documentation and palette contributions are welcome. Contributions to original project code are under MIT.

## Development

Use Windows x64, Rust with the MSVC target, and Microsoft C++ Build Tools with a Windows SDK. The pinned Rust toolchain includes rustfmt and Clippy. Clone your fork, open PowerShell in its root, and run:

```powershell
.\BUILD.ps1 fmt
.\BUILD.ps1 test
.\BUILD.ps1 clippy
.\BUILD.ps1 release
.\BUILD.ps1 package
```

The scripts do not install application dependencies globally. `cargo fmt` applies formatting; `BUILD.ps1 fmt` only checks it. CI repeats checks on Windows and checks the declared minimum Rust version. A successful CI run does not establish physical keyboard compatibility.

## Code map

| Location | Responsibility |
| --- | --- |
| `src/app.rs` | Native screens, draft state, command worker and close handling |
| `src/device.rs`, `src/protocol.rs` | HID adapter, packet validation, readback and macro transactions |
| `src/macros.rs`, `src/profiles.rs` | Macro encoding and validated profile persistence |
| `src/audio_capture.rs` | Windows playback loopback |
| `src/music.rs`, `src/music_dsp.rs`, `src/live_rgb.rs` | Audio worker, effects, palettes and temporary USB streaming |
| `src/scene.rs`, `src/keyboard_scene.rs`, `src/layout.rs` | Camera, keyboard rendering and hit testing |
| `assets/keyboard-layout.json` | Included physical layout and matrix identities |
| `src/bin/f75-check.rs` | Explicit diagnostic and recovery commands |

To add a palette, extend `Palette` and its metadata and color interpolation in `src/music_dsp.rs`, register it in `Palette::ALL`, and extend gradient boundary and selection tests. The picker, swatches and live frames share those colors. Keep existing names and gradients stable.

Use `node tools/import-layout.mjs "C:\path\to\KB.ini"` only when deliberately regenerating geometry from a configuration you have permission to use. Node is not required to build or run the app. Review matrix changes carefully: a visible position must remain associated with the correct physical key.

## Testing

The normal suite covers protocol boundaries, malformed data, macro rollback faults, profile persistence, audio analysis, rendering geometry and native UI interactions. It uses synthetic inputs. Explicitly ignored USB tests write to physical hardware and must remain opt-in.

To inspect a connected USB keyboard without changing it:

```powershell
.\BUILD.ps1 probe
.\target\release\f75-check.exe --macros
```

For a deliberately authorized hardware test, close all keyboard software, use wired USB, retain its recovery checkpoint, and read the test before executing it:

```powershell
cargo test --locked --bin aula-studio usb_profile_apply_preserves_the_current_hardware_state -- --ignored --nocapture --test-threads=1
cargo test --locked --bin aula-studio usb_music_sync_restores_the_current_hardware_state -- --ignored --nocapture --test-threads=1
```

The music test needs playback through the default Windows playback endpoint. Never run hardware tests automatically in CI or with unapproved devices. Do not use forced process termination to end music tests.

## Pull requests

Keep changes focused. Explain the observable problem, resulting behavior, checks run and remaining limitations. Include a regression test for a reproducible bug. For UI changes, inspect actual rendered windows at 1440x900, 1200x780 and 920x650, including keyboard navigation.

Do not commit local profiles, recovery snapshots, audio, credentials, OEM binaries, proprietary disassembly, `target`, `.tools`, `verification` or `dist`. Release archives are generated from an explicit file list. Never attach a recovery checkpoint to a public issue: it may contain personal key assignments and macros.

Dependency changes require a lockfile update, a fresh RustSec audit and regenerated applicable notices. Use the exact Cargo metadata and the notice generation script described in `THIRD_PARTY_NOTICES.md`. A platform-specific build must include that platform's dependency licenses.
