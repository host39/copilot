//! `file`: project files and chat attachments (allowlist, magic bytes, quota, token estimate).

mod capacity;
mod entities;
mod media;
mod ports;
mod service;

pub use crate::domains::file::capacity::{Capacity, CapacityLevel, largest};
pub use crate::domains::file::entities::{
    FileOwner, Measured, StoredFile, StoredFileParts, file_name,
};
pub use crate::domains::file::media::{
    FileKind, MediaType, OCTET_STREAM, estimate, pdf_pages, served_type,
};
pub use crate::domains::file::ports::FileRepository;
pub use crate::domains::file::service::{FileLimits, FileService, FileUpdate};
