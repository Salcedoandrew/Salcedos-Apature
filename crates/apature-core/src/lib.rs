//! Core models shared by every other crate: what a photo *is* on disk, how
//! RAW+JPEG pairs are grouped, the culling marks a photographer applies, and a
//! small TIFF/IFD reader that both preview extraction and metadata use.
//!
//! This crate has no I/O beyond directory scanning and never touches image
//! pixels.

pub mod fileops;
pub mod folder;
pub mod item;
pub mod marks;
pub mod tiff;

pub use folder::{scan, ScanOptions};
pub use item::{Kind, PhotoItem, Variant};
pub use marks::{ColorClass, ColorClassScheme, Marks, Rating};

/// Apature's own folders (cache, settings, log, ingest ledger): one place
/// for the `dev.mccal.Apature` identity. None without a home folder.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "mccal", "Apature")
}
