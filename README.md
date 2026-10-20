# OpenFUSE

[![Status](https://img.shields.io/badge/status-experimental_preview-orange)](#status)
[![Rust](https://img.shields.io/badge/built_with-Rust-dea584)](Cargo.toml)
[![CI](https://github.com/dbarbashov/openfuse/actions/workflows/rust.yml/badge.svg)](https://github.com/dbarbashov/openfuse/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

**A native Rust editor for the amplifiers you still want to play.**

Open, edit and organize legacy Fender FUSE presets. Keep your collection on your
computer and work offline, without Silverlight, Mono or a Fender account.

[Download v0.1.0](https://github.com/dbarbashov/openfuse/releases/tag/v0.1.0) ·
[Watch the feature tour](https://dbarbashov.github.io/openfuse/) ·
[User guide](docs/usage.md) · [Build from source](#build-from-source)

![The OpenFUSE native preset editor](docs/media/editor.png)

> [!WARNING]
> **Experimental, AI-assisted software — use at your own risk.** Substantial parts
> were developed with coding agents (“vibe-coded”). This is not an audited or
> production-ready product. No amplifier model is hardware-validated by this
> release. Work on copies and keep an independent backup before device writes.
> Read the [full disclaimer](DISCLAIMER.md).

## See it in action

[![Native editing, model selection and library interaction](docs/media/openfuse-preview.gif)](https://github.com/dbarbashov/openfuse/releases/download/v0.1.0/openfuse-demo.mp4)

**[30-second feature tour](https://dbarbashov.github.io/openfuse/)** ·
[Download the MP4](https://github.com/dbarbashov/openfuse/releases/download/v0.1.0/openfuse-demo.mp4)

Real native interactions show parameter editing, model selection, bypass, undo,
library search and expression ranges. USB scenes use a simulator. The original
editorial soundtrack is not amplifier audio.

## Features

- **Preset editor:** amplifier and effect models, model-aware parameters, chain
  arrangement, bypass and undo/redo. Sound, Expression and Details inspectors.
- **Preset preservation:** XML import/export, unknown-field retention where
  supported, metadata, colors and local preset collections.
- **Expression controls:** parameter assignments, heel/toe ranges and selective
  module import with a compatibility preview.
- **Backups and banks:** historical backup-folder import, selected-slot mapping,
  FX/VOICE bank editing and explicit-root G-DEC file tools.
- **Experimental device workspace:** reads, temporary audition, live parameters,
  tuner, stored-preset saves and bank restore with explicit confirmation.
- **Practice tools:** local WAV/MP3 playback, preset track links and audio-output
  selection, sample rate and buffer controls.
- **Appearance:** System, Light and Dark themes in a native Rust/egui application.

| Expression assignments | Local preset library |
| --- | --- |
| ![Expression editor](docs/media/expression.png) | ![Local library](docs/media/library.png) |

| Light appearance | Dark appearance |
| --- | --- |
| ![Light editor](docs/media/editor.png) | ![Dark editor](docs/media/editor-dark.png) |

## Download and install

The [v0.1.0 prerelease](https://github.com/dbarbashov/openfuse/releases/tag/v0.1.0)
provides an **Apple Silicon macOS DMG**, source archive with vendored Rust
dependencies, demo video and checksums.

Open the DMG and copy OpenFUSE.app to Applications. The app is ad-hoc signed,
**not notarized or Developer ID signed**. macOS may require its normal
Privacy & Security / Open Anyway flow. Only proceed if you trust the source;
there is no need to disable Gatekeeper globally.

Linux, Windows and Intel Mac users can build from source. This release does not
provide validated binaries for those platforms. See [packaging](packaging/README.md).

## Build from source

Requires Rust 1.89+ and native platform build tools. On Debian/Ubuntu, install
`libasound2-dev`, `libudev-dev`, `pkg-config` and `libxkbcommon-dev`.

```sh
git clone https://github.com/dbarbashov/openfuse.git
cd openfuse
cargo run --release --locked -p openfuse-desktop
```

Enable experimental USB HID access explicitly:

```sh
cargo run --release --locked -p openfuse-desktop --features hid
```

The app starts offline. Enumeration and connection require explicit actions.
For an offline demonstration, append `-- --demo`. Run the CLI with
`cargo run --locked -p openfuse-cli -- --help`.

The release source archive includes Cargo dependencies. From its extracted root:

```sh
cargo build --offline --locked --release -p openfuse-desktop --features hid
```

## Status

The editor contains catalogs for G-DEC 3, Mustang V1, Mustang V2, Mustang Floor,
Bronco, Super Champ X2 and Passport Mini preset families. Catalog coverage and
USB identity recognition do **not** establish hardware compatibility.

Automated tests and simulator checks cover many editing and protocol paths.
Physical reads, sound, reconnect, persistent writes and restore still need
exact-model/firmware validation. Firmware updates/recovery, phrase sampling,
G-DEC mode/mount automation, full accessibility/localization and discontinued
community services remain outside this preview's complete support.

OpenFUSE edits device presets; it does not emulate the amplifier's DSP.

## Contributing

Bug reports and focused fixes are welcome. Include the version, OS, exact device
and firmware when relevant, and a minimal synthetic or redacted example.
See [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities through
[private reporting](https://github.com/dbarbashov/openfuse/security/advisories/new).

## License

Original OpenFUSE code is [MIT licensed](LICENSE). Dependencies and fonts retain
[their own licenses](THIRD_PARTY.md). No Fender source code, decompiled source,
installers, assemblies, firmware, artwork or preset collections are distributed.

This independent project is not affiliated with or endorsed by Fender. Product
names identify compatibility targets. OpenFUSE is not represented as a
clean-room implementation. See [DISCLAIMER.md](DISCLAIMER.md).
