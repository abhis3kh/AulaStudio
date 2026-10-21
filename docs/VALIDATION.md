# Release validation record

This records local preparation of Aula Studio 0.1.0 for Windows x64. Automated checks were repeated on **2026-10-21** with Rust **1.99.0**. Hardware and clean-source checks below retain their actual **2026-10-20** dates. No GitHub repository, commit, tag or hosted CI run is implied by these results.

## Automated checks

Commands run from the project root in PowerShell:

| Command | Result |
| --- | --- |
| `.\BUILD.ps1 fmt -Offline` | Passed. Formatting check only. |
| `.\BUILD.ps1 test -Offline` | Passed: 109 tests, 0 failures. Two explicit USB hardware tests ignored. |
| `.\BUILD.ps1 clippy -Offline` | Passed with `-D warnings` across all targets and features. |
| `python tools/generate-notices.py --verify-bundle` | Passed: hashes and inventory for 143 Windows Cargo packages, embedded font notices and the Rust distribution notices. |
| `.\BUILD.ps1 package -Offline` | Passed. Built both executables with default features and created the Windows x64 ZIP and SHA-256 file. |

The test total is 24 library tests, 75 application tests and 10 recovery-utility tests. Coverage includes protocol bounds, malformed audio, palette interpolation, profile size limits, unsafe macro references, injected macro transaction failures, checkpoint recovery, keyboard hit testing, palette interactions, pending music startup and closing during an in-flight hardware command. These are synthetic tests unless explicitly described as hardware checks below.

`BUILD.ps1 test` expands to `cargo test --all-targets --all-features --locked`; Clippy uses `cargo clippy --all-targets --all-features --locked -- -D warnings`. `-Offline` adds Cargo's `--offline` flag. The test process used a writable temporary directory under `target`. The local H: filesystem produced an incremental-cache hard-link warning; Rust copied cache files instead. There were no Clippy code diagnostics or test failures.

## Clean-source contributor checks

A copy containing only intended public repository files, with no project-local `.tools` directory, passed these checks on 2026-10-20:

| Toolchain and command | Result and scope |
| --- | --- |
| Rust 1.99.0, `.\BUILD.ps1 check -Offline` | Passed using installed Cargo, all targets and all features. |
| Rust 1.95.0, `cargo check --locked --offline --all-targets` | Passed with default features. Confirms the declared minimum for this source and lockfile. |

These checks reused an existing Cargo registry cache and installed compiler tools. They verify independence from the ignored local toolchain, not installation on a completely fresh Windows machine. The source hashes for `app.rs`, `device.rs`, `profiles.rs` and `BUILD.ps1` still matched that clean-copy record when checked on 2026-10-21.

## Real USB checks

The following opt-in tests passed on 2026-10-20 using one wired F75. They were not repeated during the final packaging check. Both saved a complete recovery checkpoint before writes.

```powershell
cargo test --locked --bin aula-studio usb_profile_apply_preserves_the_current_hardware_state -- --ignored --nocapture --test-threads=1
cargo test --locked --bin aula-studio usb_music_sync_restores_the_current_hardware_state -- --ignored --nocapture --test-threads=1
```

- The profile test applied an unchanged hardware profile through the application worker and verified complete settings, RGB data, both keymaps and the 4096-byte macro bank against the saved original.
- The music test captured 140 analyzed audio frames at 48,000 Hz, accepted 32 temporary USB frames, exercised all three music modes, and reported a peak onset value of 1.0. Stop completed successfully and exact readback confirmed restoration of settings, RGB data, both keymaps and the macro bank. No failure was reported.

These results prove the exercised transfers and readback on that device. They do not prove physical LED appearance, every firmware revision, wireless configuration, or restoration after power loss and forced termination. Personal checkpoints and raw device state remain local and are excluded from the repository and release archive.

## Native window check

On 2026-10-21 the default-feature release opened on the Windows **Default** desktop with the title `Aula Studio | F75`, a responsive 1440 by 900 client area and USB Online status. The rendered keyboard and controls were visually inspected. The actual application window closed normally. No hardware settings were changed by this launch check.

This checks startup, the rendered initial screen, USB connection and normal close. It is not a fresh end-to-end run of every screen or a sustained performance benchmark. Earlier UI work exercised 1440 by 900, 1200 by 780 and 920 by 650 layouts; those dimensions also retain synthetic layout tests.

## Dependency audit

On 2026-10-21, cargo-audit 0.22.2 checked the full lockfile against RustSec database commit `4b587f00996ebe5c05a73eaef105ef241e876414` using:

```powershell
cargo-audit audit --db .tools/advisory-db --json
```

The audit covered 348 lockfile dependencies and reported **0 known vulnerabilities** and three **unmaintained** warnings:

| Advisory | Package |
| --- | --- |
| RUSTSEC-2026-0370 | `zvariant 5.15.0` |
| RUSTSEC-2026-0371 | `zbus_names 4.3.4` |
| RUSTSEC-2026-0372 | `zvariant_derive 5.15.0` |

None of these three packages appears in the resolved Windows default-feature dependency graph used for release notices. They remain in the cross-platform lockfile and the warnings are not suppressed. This is a dated advisory database result, not a guarantee of security. Rerun the audit before future releases and after dependency changes.

## Release scope and remaining verification

The packaged executables use default features. Screenshot capture and experimental wireless diagnostic features are excluded. The package includes the recovery utility, MIT license, applicable dependency and font licenses, and these limitations.

The archive was opened and its CRCs checked. All 317 entries matched the explicit release file list and current source files byte for byte. This includes 306 license files and manifests. The verifier checked 305 individual notice hashes, the notice index hash, the lockfile hash, the archive SHA-256 file and 304 relative documentation links. No private checkpoints, developer toolchains, source backups or local diagnostic output were included.

The verified executable SHA-256 values are:

| Executable | SHA-256 |
| --- | --- |
| `AulaStudio.exe` | `ab7f0d8e1b661607bd08bccf3cc6c1b0cdfd68d6ba87da0e3a8bff90d8650d5b` |
| `f75-check.exe` | `2515ca0b5bdef3a942279226f9c64a77c1c8d501fee5d0806d51196bd2564b63` |

The ZIP has its own adjacent `.sha256` file. Repackaging changed documentation can change the ZIP hash while leaving these executable hashes unchanged.

The release is unsigned. Windows 10, other keyboard revisions, other layouts, arbitrary driver failure, physical unplug/replug during writes, power loss, and long-duration performance have not been verified by this preparation. Multi-report operations cannot be made atomic by these tests. See [RELIABILITY.md](RELIABILITY.md).

GitHub Actions configuration is included, but hosted Windows checks and the dependency-audit workflow have **not run** because publication is left to the repository owner. Complete the publication steps in [PUBLISHING.md](PUBLISHING.md), then inspect the first hosted results before publishing a downloadable release.
