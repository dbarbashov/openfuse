# Third-party material

The project license covers project-authored material only. Upstream software,
fonts and notices retain their original licenses. No Fender installer, assembly,
firmware, artwork, decompiled source or original preset collection is shipped.

## Native application

Rust dependencies are pinned in Cargo.lock. Each binary package includes
`third-party/inventory.json`, the exact published `.crate` source archives of its
built dependencies, and their license/notice files. MPL-2.0 Symphonia sources are
included. The release source archive also supplies vendored Cargo dependencies
for offline compilation of the native application.

Some crates omit standalone license files. Reviewed supplements under
`packaging/third-party/` record the exact crate hash, upstream VCS revision,
source URL and license-file hash; packaging checks these bindings before copying.
These additions do not relicense any upstream work. They are an attribution
inventory, not a legal certification.

The native fallback fonts bundled by epaint keep their own MIT/OFL/Ubuntu Font
licenses. The app may read the operating system's installed UI font; macOS system
font files are not redistributed. The amplifier icon in `packaging/openfuse.svg`
is project-authored.

## Demo and documentation assets

Native screenshots show independently authored examples or simulated devices.
The editorial video score is project-authored synthesized music, not Fender DSP
output. Inter and Space Grotesk video fonts retain the SIL Open Font License
notices in `docs/media/licenses/`. Video-production tools are not part of the
native application runtime.

## Product names

Product and model names identify compatibility targets and do not imply
sponsorship, endorsement or a grant of trademark rights.
