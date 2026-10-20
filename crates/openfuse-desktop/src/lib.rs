//! Browser entry point, sharing the native editor and lossless preset model.
#![cfg(target_arch = "wasm32")]
// The native editor uses additional session/theme helpers outside the web subset.
#[allow(dead_code)]
mod session;
#[allow(dead_code)]
mod theme;
mod views;
mod web;
