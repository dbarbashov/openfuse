# User guide

## Start offline

Launch OpenFUSE and open a copy of a `.fuse` preset. No amplifier is needed for
local editing. Use the built-in example with `openfuse-desktop --demo`.
The application starts offline; listing devices and connecting are explicit actions.

## Edit a preset

Choose an amplifier or effect block in Sound, select its model and adjust the
available controls. Unsupported fields remain visible where possible. Use Undo
and Redo to compare changes, then Save As to create a separate variation.
Expression contains parameter assignments and heel/toe ranges. Details contains
preset metadata and colors. Choose System, Light or Dark appearance from More.

## Manage a collection

Select a local library folder to search, sort, import and duplicate presets.
Archive entries to put them aside recoverably. Do not edit files inside the
read-only disk image. Keep independent copies of important presets and backups.

## Use device features carefully

USB access requires a build with the `hid` feature. Choose the intended device
and inspect its identity before connecting. A listed device is not a support
claim. Prefer read-only inspection before any audition or persistent operation.
Temporary audition and stored-preset/bank writes are different operations.
Review the source, destination and operation preview before confirming a write.
Cancel is not a guarantee of rollback after a write has reached the device.

No physical amplifier model is validated by this preview. Firmware updates and
recovery are not supported. See the [disclaimer](../DISCLAIMER.md).

## Practice audio

Choose a local backing-track folder and play supported WAV/MP3 files. Audio
settings choose the computer's output, sample rate and buffer where available.
This is local file playback; OpenFUSE does not recreate an amplifier's DSP.

## Installation and troubleshooting

See [portable packages](../packaging/README.md) for platform setup, file
associations and preference locations. Both binaries support `--help` offline.
For a problem report, include your OS, version, device/firmware if relevant and
a minimal example without personal data or proprietary material.
