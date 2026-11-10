//! The sync engine: verifies the signed manifest, downloads only what changed, and applies it.

pub mod lock;
pub mod selfupdate;
pub mod source;
pub mod sync;

pub use lock::Lock;
pub use source::{DirSource, HttpSource, Source};
pub use sync::{apply, check, stage, sync, Check, Pending, Progress, Report, SyncOptions};
