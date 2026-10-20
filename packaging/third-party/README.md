# Supplemental dependency notices

This directory supplements license files missing from exact published Cargo
archives. It does not contain Fender material or application source from any
reverse-engineering target.

`supplemental.json` binds each package/version to its `.crate` SHA-256 and the
upstream VCS commit recorded in `.cargo_vcs_info.json`. Each notice has a source
URL and SHA-256. Packaging remains offline and rejects mismatched hashes or paths
outside this directory.

Most files are unmodified notices from those pinned upstream revisions. Two
explicit exceptions are documented in the manifest and file headers:

- `dispatch 0.2.0`: its exact Cargo.toml declares MIT, but the pinned repository
  has no standalone license file. Its upstream declaration and author metadata
  are retained alongside standard MIT permission/warranty text.
- Newer `objc2` revisions: upstream LICENSE.md declares the applicable licenses
  and discusses Apple SDK provenance. That statement is preserved verbatim;
  standard MIT text is supplied separately for its MIT option.

These standard-text supplements are not represented as upstream-authored files.
Original authors and all source notices remain in the exact bundled source
archives. Nothing here asserts a legal certification or resolves upstream
licensing questions beyond recording their published declarations.

Font licenses in the original `epaint_default_fonts` crate are copied directly
by the packager, including the font-specific text files.
