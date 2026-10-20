# Portable builds

Build on the destination OS with Rust, Python 3.11+ and its native SDK:

```
python3 tools/package_release.py
```

The command builds the desktop application and offline CLI in release mode with
`--locked --features hid`. It stages only an explicit allowlist under
`target/distribution/`, records source/binary hashes and native dependencies, then
creates a macOS DMG, Linux tar.gz or Windows ZIP. It refuses an existing output
directory; use `--output` for another build. No install, association change,
device enumeration, USB connection or publication happens during packaging.
The macOS check mounts its new DMG read-only, verifies all packaged hashes and
the ad-hoc signature, runs both embedded binaries with `--help`, then unmounts it.

The bundle contains the project license, user guide, dependency inventory and exact
registry dependency source archives, including MPL-licensed Symphonia sources.
Source archives retain upstream notices; license files found in those sources
are also collected separately. Crates that omit standalone notices receive hash-bound supplements from
`packaging/third-party/`. Font notices are retained. The dispatch 0.2.0 supplement
explicitly identifies standard MIT text separately from its upstream license
declaration. Packaging refuses a dependency with neither notices nor a reviewed
supplement. This inventory is not a legal certification. There are no Fender binaries, firmware or artwork.
The small amplifier icon is original OpenFUSE SVG artwork, covered by the
repository license; macOS ICNS variants are rendered by system tools.
Only user-facing installation/usage guides are bundled. Development reports,
research files and proprietary reference inputs are excluded.

The manifest describes the platform and architecture actually built. Cross
compilation is not supported by this script. SHA256SUMS covers packaged files;
the outer release manifest covers the final archive and signed application.
Code signing timestamps and archive metadata mean builds are not advertised as
byte-for-byte reproducible.

## Platform behavior

- macOS: `OpenFUSE.app`, embedded CLI in `Contents/MacOS/openfuse`, resources and
  notices inside the app. The app is ad-hoc signed; it is **not notarized or
  Developer ID signed**. A downloaded build may be blocked by Gatekeeper; use
  the normal macOS Open/Privacy & Security controls only if you trust its source.
  Drag the app to a location you choose. `.fuse` is declared as an alternate
  document handler, not forcibly made the default. Minimum OS is set by the
  actual Rust build target's deployment target (currently macOS 11 on arm64).
- Linux: unpack in a stable location and run `bin/openfuse-desktop`. ALSA,
  libudev, a functioning Wayland/X11 desktop and OpenGL are required. Run
  `python3 register-desktop.py install` explicitly to register the current
  installation for your user; it does not make OpenFUSE the default handler.
  Run `python3 register-desktop.py uninstall` before moving/removing it.
  USB permissions depend on the distribution. If needed, an administrator can
  install the documented udev rule for Fender HID devices; the package never
  changes permissions itself. A recognized device is not validated support.
- Windows: unpack to a stable location, then launch `bin/openfuse-desktop.exe`.
  Run `powershell -File register-file-type.ps1 -Action Install` explicitly to
  offer OpenFUSE in Open With for `.fuse` in the current user account. Uninstall
  with `-Action Uninstall` before moving it. The helper does not override the
  user's default application. Windows native audio uses WASAPI; no ASIO driver
  is bundled. The Microsoft Visual C++ runtime may be required by the target.

`--help` works offline for both binaries. Native application launch, file-open
delivery, sound output and real USB behavior need testing on each destination
OS. Successful CI compilation/archive creation is not that hardware test.

## Settings and upgrades

The window and package are named OpenFUSE. Preferences deliberately retain the
legacy `Confuse — Preset Editor` storage path through an explicit persistence
file, independently of the new OS application identity. Audio output choice,
window state and selected library folders remain available:

- macOS: `~/Library/Application Support/Confuse-—-Preset-Editor`.
- Linux: `$XDG_DATA_HOME/confuse—preseteditor`, or
  `~/.local/share/confuse—preseteditor` when XDG_DATA_HOME is unset.
- Windows: `%APPDATA%\Confuse — Preset Editor\data`.

Preset/media/backup libraries remain in directories explicitly chosen by the
user. Replacing a portable package does not move, reset or delete those
libraries or the preference directory. Do not edit preset files inside the
read-only macOS disk image. Copy the app to a stable location before registering
it as a document handler; unregister Linux/Windows helpers before moving their
installation directory.

## CI

The manually triggered release workflow builds three platform artifacts and
runs packaging tests plus offline `--help` smoke checks. It uploads workflow
artifacts only; it never creates or publishes a GitHub release. Actual CI runs
and Windows/Linux launch evidence must be checked separately from local macOS
validation.

The Linux Exec escaping follows the
[freedesktop Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html).
