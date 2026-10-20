//! Original, offline building blocks for the openfuse preservation project.
//!
//! Raw byte inspection deliberately assigns no undocumented protocol meaning.

pub mod catalog;
pub mod hex;
#[cfg(not(target_arch = "wasm32"))]
pub mod library;
pub mod preset;
pub mod protocol;
pub mod report;

pub mod messages;

pub mod model_catalog;

pub mod usb_session;

pub mod wire_preset;

pub mod audition;

#[cfg(not(target_arch = "wasm32"))]
pub mod archive;
#[cfg(not(target_arch = "wasm32"))]
pub mod auxiliary;
pub mod bank;

pub mod controls;

#[cfg(not(target_arch = "wasm32"))]
pub mod legacy_backup;

#[cfg(not(target_arch = "wasm32"))]
pub mod control_preset;

#[cfg(not(target_arch = "wasm32"))]
pub mod control_library;

#[cfg(not(target_arch = "wasm32"))]
pub mod preset_file;

pub mod conversion;
#[cfg(not(target_arch = "wasm32"))]
pub mod media_library;

#[cfg(not(target_arch = "wasm32"))]
pub mod mounted_storage;

pub mod media_format;
