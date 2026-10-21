# Reliability and verification

## Behavior and safeguards

- The worker serializes device operations and holds an exclusive app lease. Initial connection reads state.
- User edits change a local draft. Explicit Apply actions write and verify state.
- Normal close waits for an active device operation. A failed operation keeps the error visible.
- Macro replacement captures settings, colors, both maps and the complete macro bank before writing. It clears macro references before replacing the bank, verifies the complete bank and installs assignments afterward. Failure attempts verified rollback and retains the recovery checkpoint.
- A saved profile must contain the definitions needed by its macro references before it can be applied. Legacy profiles remain readable, with unsafe applications refused before writes.
- Profile reads and writes share a 2,000,000-byte limit. Invalid or oversized saves do not replace the previous file.
- Music has exclusive ownership of USB writes while active. Latest-frame buffering bounds queued work and streaming is capped at 20 Hz. Stop and normal close restore settings and colors and verify maps.
- Recovery checkpoints are written to disk before temporary music changes or macro-bank replacement. Keep them until recovery has been verified.

## Limits

These safeguards do not make multiple HID reports atomic. Cable disconnection, forced termination or power loss can leave a partial operation. Ordinary profile application is verified step by step, but the entire profile is not an atomic transaction and earlier successful fields can remain applied after a later failure.

Windows HID feature calls can block in the driver. The application cannot guarantee a finite timeout or restoration after a driver hang. The UI can remain responsive while waiting for a worker, but terminating it forfeits automatic restoration.

Runtime testing covers one wired F75 and the included layout. It does not establish compatibility with every F75 revision, Windows version, USB hub, graphics driver or keyboard layout. Wireless writes are disabled in ordinary builds after unsuccessful earlier receiver tests. Bluetooth configuration is unsupported.

Music uses the playback endpoint selected at Start. Endpoint changes require restarting capture. No physical camera verification of emitted LED colors has been performed. On-screen colors and USB readback are separate evidence.

The release is unsigned. Performance depends on the GPU, audio endpoint and active scene. Short idle measurements are not sustained CPU, memory or frame-rate benchmarks.

## Release checks

See [VALIDATION.md](VALIDATION.md) for the commands and results recorded for this preparation. GitHub Actions is provided but cannot be called passed until it runs in the published repository.

Ordinary tests use synthetic data and do not exercise Windows driver failures. The explicitly ignored integration tests require real USB access. Keep hardware tests separate from unattended CI and preserve their recovery checkpoint.

## Dependency audit

The 2026-10-21 RustSec check of the complete lockfile found no reported vulnerabilities. It reported three maintenance warnings: `RUSTSEC-2026-0370`, `RUSTSEC-2026-0371` and `RUSTSEC-2026-0372` for `zvariant`, `zbus_names` and `zvariant_derive`. These are transitive platform dependencies retained in the lockfile and are absent from the Windows build graph. Warnings are retained and not suppressed. An audit is a dated database check, not a guarantee that vulnerabilities do not exist.
