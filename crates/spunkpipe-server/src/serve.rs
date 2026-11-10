//! A small built-in web server for a published folder, for people with no web server.
//! It only serves the files a client needs (`latest`, `manifests/`, `blobs/`), read-only, with
//! range requests so interrupted downloads can resume. Put a reverse proxy in front for TLS.

use spunkpipe_core::path::validate_rel_path;
use spunkpipe_core::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub struct Running {
    server: Arc<Server>,
    workers: Vec<JoinHandle<()>>,
    pub addr: std::net::SocketAddr,
}

impl Running {
    /// Stops accepting requests and waits for the workers.
    pub fn stop(self) {
        for _ in &self.workers {
            self.server.unblock();
        }
        for w in self.workers {
            let _ = w.join();
        }
    }

    /// Blocks until the server is stopped (it normally never is).
    pub fn wait(self) {
        for w in self.workers {
            let _ = w.join();
        }
    }
}

pub fn start(root: PathBuf, listen: &str, threads: usize) -> Result<Running> {
    if !root.is_dir() {
        return Err(Error::Other(format!("{} is not a directory", root.display())));
    }
    let server = Server::http(listen).map_err(|e| Error::Other(format!("cannot listen on {listen}: {e}")))?;
    let addr = server.server_addr().to_ip().ok_or_else(|| Error::Other("not an IP socket".into()))?;
    let server = Arc::new(server);
    let root = Arc::new(root);
    let workers = (0..threads.max(1))
        .map(|_| {
            let (server, root) = (server.clone(), root.clone());
            std::thread::spawn(move || {
                while let Ok(req) = server.recv() {
                    handle(req, &root);
                }
            })
        })
        .collect();
    Ok(Running { server, workers, addr })
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

/// Only these paths are ever served.
fn allowed(path: &str) -> bool {
    if validate_rel_path(path).is_err() || path.split('/').any(|c| c.starts_with('.')) {
        return false;
    }
    path == "latest" || path.starts_with("manifests/") || path.starts_with("blobs/")
}

/// `bytes=N-` (the only form clients send) -> N.
fn range_start(req: &Request) -> Option<u64> {
    let h = req.headers().iter().find(|h| h.field.equiv("Range"))?;
    h.value.as_str().strip_prefix("bytes=")?.strip_suffix('-')?.parse().ok()
}

fn handle(req: Request, root: &std::path::Path) {
    let head = *req.method() == Method::Head;
    if !matches!(req.method(), Method::Get | Method::Head) {
        let _ = req.respond(Response::empty(405).with_header(header("Allow", "GET, HEAD")));
        return;
    }
    let url = req.url().split('?').next().unwrap_or("").trim_start_matches('/').to_string();
    if !allowed(&url) {
        let _ = req.respond(Response::empty(404));
        return;
    }
    let mut path = root.to_path_buf();
    path.extend(url.split('/'));
    let Ok(mut file) = File::open(&path).and_then(|f| f.metadata().map(|m| (f, m))).and_then(|(f, m)| {
        if m.is_file() {
            Ok(f)
        } else {
            Err(std::io::ErrorKind::NotFound.into())
        }
    }) else {
        let _ = req.respond(Response::empty(404));
        return;
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let cache = if url == "latest" { "no-cache" } else { "public, max-age=31536000, immutable" };
    let start = range_start(&req);
    let (status, body_len, content_range) = match start {
        Some(s) if s >= total && total > 0 => {
            let _ = req.respond(Response::empty(416).with_header(header("Content-Range", &format!("bytes */{total}"))));
            return;
        }
        Some(s) if s > 0 => {
            if file.seek(SeekFrom::Start(s)).is_err() {
                let _ = req.respond(Response::empty(500));
                return;
            }
            (206, total - s, Some(format!("bytes {s}-{}/{total}", total - 1)))
        }
        _ => (200, total, None),
    };
    let reader: Box<dyn Read + Send> = if head { Box::new(std::io::empty()) } else { Box::new(file) };
    let mut resp = Response::new(StatusCode(status), vec![], reader, Some(body_len as usize), None)
        .with_header(header("Content-Type", if url.ends_with(".json") || url == "latest" || url.ends_with(".sig") { "text/plain" } else { "application/octet-stream" }))
        .with_header(header("Accept-Ranges", "bytes"))
        .with_header(header("Cache-Control", cache))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    if let Some(cr) = content_range {
        resp = resp.with_header(header("Content-Range", &cr));
    }
    let _ = req.respond(resp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_client_paths_are_allowed() {
        for ok in ["latest", "manifests/3.json", "manifests/3.sig", "blobs/ab/abcd.zst"] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in ["", "..", "../etc/passwd", "blobs/.tmp/x", ".publish-cache.json", "other.txt", "blobs/../latest", "manifests//x", "/latest"] {
            assert!(!allowed(bad), "{bad}");
        }
    }
}
