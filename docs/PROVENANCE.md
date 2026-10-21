# Provenance

## Original implementation

The Rust application and rendering code are independently implemented. The source distribution does not contain OEM executables, firmware, decompiler output, proprietary artwork, personal profiles or keyboard recovery snapshots.

## Hardware layout data

`assets/keyboard-layout.json` records numeric key rectangles, key labels, virtual-key values and matrix identities from the ISO/French F75 `KB.ini` configuration. The importer reads the user's supplied configuration path. The original configuration is not redistributed.

The 82 entries include a zero-area Wheel control. Rendering and selection exclude this non-key entry. The visible key geometry is normalized by `src/layout.rs` while preserving matrix identities. A different keyboard variant requires separate physical validation.

## Protocol reference

The public [aula-f75-linux protocol notes](https://github.com/trenbolone1122/aula-f75-linux/blob/main/docs/PROTOCOL.md) informed packet research together with local device observations. No source code or artwork from that repository is included. It had no visible root license when inspected on 2026-10-20. Do not assume this project's MIT license grants permission to redistribute another project's material.

## Dependencies and tools

Cargo dependencies and fonts retain their original licenses. `THIRD_PARTY_NOTICES.md` and `licenses/` cover the recorded Windows dependency graph. Rust and Microsoft build/runtime components have separate notices and terms described there.

GitHub CLI, RustSec tooling, local diagnostic captures and development toolchain installations are development resources and are excluded from the source and application archives.

Screenshots in `docs/screenshots` show this project's own interface. They are illustrative captures, not proof of physical LED appearance or support for other firmware.
