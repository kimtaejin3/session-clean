//! 정리 트랜잭션과 휴지통.

pub mod cleanup;
pub mod fsutil;
pub mod history;
pub mod manifest;
pub mod trash;

pub use cleanup::{CleanupOutcome, CleanupPreview, CleanupTarget, SkipReason};
pub use manifest::{CleanupMode, Manifest, ManifestFile, ManifestSession, OpStatus, SharedRecord};
