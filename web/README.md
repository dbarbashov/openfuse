# Browser editor

The WebAssembly application shares `session.rs`, `views.rs`, expression/details
panels and the core preset model with the desktop app. Native filesystem,
USB and audio dependencies are excluded from the browser target.

Supported: opening UTF-8 XML `.fuse` presets up to 1 MiB, seven new-preset
families, amplifier/effect models, parameters, bypass, ordering, expression,
metadata/colors, undo/redo and browser downloads. Unknown fields follow the same
preservation rules as the native parser. Invalid imports keep the current preset.

Files are processed locally. No account, analytics or upload endpoint is used.
The draft lives in the current tab: download it before navigating away. Browser
reload/close warnings are best effort and are not an autosave mechanism. First
loading requires a network connection; this preview does not install a service
worker. Canvas editing works best on a desktop-sized screen; accessibility is
limited compared with native HTML form controls.

USB/device operations, bank editors, conversion review, libraries, filesystem
watching and audio playback are not available in this web preview.

## Build

Requires Rust, Python 3.11+, and the wasm-bindgen CLI matching Cargo.lock:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
python3 tools/build_web.py
python3 -m http.server 8766 --bind 127.0.0.1 --directory target/web
```

Open `http://127.0.0.1:8766`. Serve over HTTP/HTTPS; opening index.html via
`file://` cannot load WebAssembly. The output includes dependency sources and
notices. GitHub Pages builds and tests this same artifact before deploying `/app/`.

## Browser checks

With the HTTP server running, use Node.js 22+:

```sh
npm ci --prefix web
npm test --prefix web
```

Set `CHROME_PATH` to use an installed Chromium/Chrome executable, or
`OPENFUSE_WEB_URL` to test another deployment. The test performs actual canvas
editing and checks exported XML, undo/redo, unknown-field roundtrips, rejected
imports, all seven new-preset families, themes, sizing and network isolation.
