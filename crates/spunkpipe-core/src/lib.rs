//! Core of SpunkPipe: manifest format, hashing, signing and the blob store.

pub mod error;
pub mod fsutil;
pub mod hash;
pub mod invite;
pub mod manifest;
pub mod par;
pub mod path;
pub mod release;
pub mod sign;
pub mod store;

pub use error::{Error, Result};
pub use hash::Hash;
pub use invite::Invite;
pub use manifest::{FileEntry, Manifest};
pub use sign::{PublicKey, SecretKey};

/// Relative location of the pointer to the newest manifest on a server.
pub const LATEST: &str = "latest";

/// Relative location of the manifest with the given serial.
pub fn manifest_path(serial: u64) -> String {
    format!("manifests/{serial}.json")
}

/// Relative location of the detached signature of the manifest with the given serial.
pub fn signature_path(serial: u64) -> String {
    format!("manifests/{serial}.sig")
}
