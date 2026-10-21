# Aula Studio

A native Windows app for configuring the **AULA F75 over USB**, built with Rust, egui and OpenGL.

This repository contains the complete Rust source code. You can modify it and compile your own Windows executable. Start with [Compile an executable](#compile-an-executable).

## Features

- Lighting effects, brightness, speed, sleep settings and individual key colors.
- Base and Fn key remapping, on-board macros, and local hardware profiles.
- Interactive 2D and perspective keyboard previews.
- Music lighting from Windows playback audio: Spectrum, Beat pulse and Rhythm wave.
- Twenty palettes, including Ocean, Lagoon, Glacier, Sunset, Peach glow and Golden hour.
- Explicit Apply controls, readback verification and recovery checkpoints for music and macro-bank replacement.

The app uses no browser runtime, account, telemetry or cloud service. Playback audio is processed in memory and is not recorded or uploaded.

## Compatibility

| Item | Support |
| --- | --- |
| Operating system | Windows 10/11, x64. Local runtime verification on Windows 11 only. |
| Keyboard | AULA F75 wired USB interface `258a:010c` |
| Layout | Included geometry comes from the ISO/French F75 layout. ANSI and other revisions are not verified. |
| Wireless | Configuration over 2.4 GHz and Bluetooth is not supported in the normal app. |
| Graphics | OpenGL-capable display driver |
| Macros | Individual base-layer key triggers. Text encoding currently uses the US keyboard layout. |

This is an independent community project, not an official AULA product. AULA and F75 names identify compatible hardware. Firmware differences may affect compatibility.

## Run

Compile the app using the instructions below, then open `target/release/aula-studio.exe`. Keep `f75-check.exe` available for recovery. No Rust installation is needed on a computer that only runs the compiled executables.

1. Connect the keyboard by USB and select wired mode.
2. Close other keyboard configuration programs.
3. Open Aula Studio. Initial connection reads the keyboard.
4. Edit a setting, then use its **Apply** control to write it.

The executable is not code-signed.

Profiles are stored in `%LOCALAPPDATA%\AulaStudio\profiles.json`. Loading a profile changes the local draft; applying it changes the keyboard. A profile containing macros replaces the keyboard's macro bank after confirmation. Existing macros cannot be preserved merely by saving a raw keymap: recreate their definitions in the app before saving a portable macro profile.

### Music sync

Play audio through the default Windows speakers or headphones, open **Music**, choose a palette and effect, then select **Start music sync**. Gain changes sensitivity, Smoothing changes decay, and Brightness changes the lighting intensity. Clear **Sync the USB keyboard** before starting for a screen-only preview.

The palette menu includes Ocean, Sunset and Studio collections, gradient previews and keyboard navigation. Palette changes take effect during the running session. Music palette choices are session settings and are not saved in hardware profiles.

Capture uses WASAPI playback loopback, not the microphone. Stop and restart music sync after changing the default playback device. Beat pulse follows audio attacks; it does not estimate BPM. Temporary keyboard frames are limited to 20 updates per second.

### Recovery

Music sync saves a checkpoint before changing the keyboard. Stop and normal window close restore and verify the saved lighting state. Macro-bank replacement saves a full checkpoint and attempts rollback on failure. Normal close waits for an active hardware operation to finish.

Disconnecting the cable, ending the process forcibly, power loss or a stalled Windows HID request can prevent completion. Keep the checkpoint reported by the app. Reconnect in wired mode, close Aula Studio and other keyboard software, then run:

```powershell
.\target\release\f75-check.exe --recover "C:\path\to\checkpoint.json"
```

App checkpoints are under `%LOCALAPPDATA%\AulaStudio\recovery\`. Diagnostic commands save checkpoints in a `verification` folder beside the current working directory. Use a checkpoint from the same keyboard. Recovery requires exact readback and reports a failure if verification does not match.

Hardware writes involve multiple reports and are not atomic. Recovery can fail after a disconnect or driver hang. Physical verification covers one wired F75; other revisions and layouts remain unverified.

## Compile an executable

### Prerequisites

On Windows x64, install:

1. [Rust using rustup](https://www.rust-lang.org/tools/install), with the default **MSVC** toolchain.
2. Microsoft Visual Studio Build Tools with **Desktop development with C++**, the MSVC x64/x86 build tools, and a Windows 10 or Windows 11 SDK.

Reopen your terminal after installation. The repository pins Rust 1.99.0 with `rust-toolchain.toml`; rustup obtains that compiler when needed. The minimum supported Rust version is 1.95. The first build needs internet access to download Rust components and the locked Cargo dependencies. No keyboard is needed to compile.

### Build and run

Clone this repository, then open PowerShell in the directory containing `Cargo.toml` and `BUILD.ps1`. Run:

```powershell
.\BUILD.ps1 release
.\target\release\aula-studio.exe
```

The build creates:

| File | Purpose |
| --- | --- |
| `target/release/aula-studio.exe` | The desktop app |
| `target/release/f75-check.exe` | The USB diagnostic and recovery utility |

You can also compile directly with Cargo:

```powershell
cargo build --locked --release --bin aula-studio --bin f75-check
```

### Check changes

```powershell
.\BUILD.ps1 fmt
.\BUILD.ps1 test
.\BUILD.ps1 clippy
```

`BUILD.ps1` uses installed Cargo, with an optional project-local toolchain fallback. It restores its environment changes when finished. Builds use `Cargo.lock`. `-Offline` works when the required crates and toolchain are already installed. Ordinary tests do not change a keyboard.

Release builds use default features. The `capture` feature is for screenshots only and must not be used for distributed builds or live hardware music tests that might exit during capture.

## License and provenance

Original project code is available under the [MIT license](LICENSE). You may modify and redistribute it while retaining the copyright and license notice. Dependencies and embedded fonts retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses](licenses/).

When redistributing compiled executables, include `LICENSE`, `THIRD_PARTY_NOTICES.md` and the `licenses/` directory. Update the third-party notices if dependencies change.

The included keyboard asset contains geometry, key labels, virtual-key values and matrix indices imported from an installed F75 configuration. It contains no OEM executable, firmware, artwork or font. Protocol behavior was independently implemented using device observations and the public [aula-f75-linux protocol notes](https://github.com/trenbolone1122/aula-f75-linux/blob/main/docs/PROTOCOL.md). That reference is attribution, not a grant to redistribute its code.
