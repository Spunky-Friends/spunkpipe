use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid hash: {0}")]
    BadHash(String),
    #[error("invalid key: {0}")]
    BadKey(String),
    #[error("signature verification failed")]
    BadSignature,
    #[error("unsafe path {0:?}: {1}")]
    UnsafePath(String, &'static str),
    #[error("invalid manifest: {0}")]
    BadManifest(String),
    #[error("invalid invite: {0}")]
    BadInvite(String),
    #[error("blob {0} is corrupt: {1}")]
    CorruptBlob(String, String),
    #[error("cancelled")]
    Cancelled,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
