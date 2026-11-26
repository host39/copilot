//! Business domains (bounded contexts), laid out as described in `README.md` next to this file.
//! Each domain's public API is re-exported from `lib.rs`.

pub mod file;
pub mod project;
pub mod provider;
pub mod settings;
