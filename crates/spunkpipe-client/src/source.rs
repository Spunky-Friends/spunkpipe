use spunkpipe_core::{Error, Result};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

/// Where a client reads the published files from.
pub trait Source: Sync {
    /// Opens `rel` starting at byte `offset`. Missing files are `Error::NotFound`.
    fn open(&self, rel: &str, offset: u64) -> Result<Box<dyn Read>>;
}

/// Reads a published directory straight from disk (used by tests and for local mirrors).
pub struct DirSource(pub PathBuf);

impl Source for DirSource {
    fn open(&self, rel: &str, offset: u64) -> Result<Box<dyn Read>> {
        let mut f = File::open(self.0.join(rel)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(rel.to_string()),
            _ => e.into(),
        })?;
        f.seek(SeekFrom::Start(offset))?;
        Ok(Box::new(f))
    }
}

pub struct HttpSource {
    base: String,
    agent: ureq::Agent,
}

impl HttpSource {
    /// `base` is the invite's base URL, with a trailing slash.
    pub fn new(base: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .user_agent(concat!("spunkpipe/", env!("CARGO_PKG_VERSION")))
            .build();
        HttpSource { base: base.to_string(), agent }
    }
}

impl Source for HttpSource {
    fn open(&self, rel: &str, offset: u64) -> Result<Box<dyn Read>> {
        let url = format!("{}{}", self.base, rel);
        let mut req = self.agent.get(&url);
        if offset > 0 {
            req = req.set("Range", &format!("bytes={offset}-"));
        }
        match req.call() {
            Ok(resp) => {
                let status = resp.status();
                let mut reader = resp.into_reader();
                if offset > 0 && status == 200 {
                    // The server ignored the range request: discard what we already have.
                    io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
                }
                Ok(Box::new(reader))
            }
            Err(ureq::Error::Status(404, _)) => Err(Error::NotFound(rel.to_string())),
            // Range starts at or past the end: the part file is already complete.
            Err(ureq::Error::Status(416, _)) => Ok(Box::new(io::empty())),
            Err(e) => Err(Error::Other(format!("{url}: {e}"))),
        }
    }
}

/// Reads a whole small file, refusing anything larger than `max` bytes.
pub fn read_all(source: &dyn Source, rel: &str, max: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    source.open(rel, 0)?.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(Error::Other(format!("{rel} is larger than {max} bytes")));
    }
    Ok(buf)
}
