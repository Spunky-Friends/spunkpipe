//! Content-addressed blob store: zstd-compressed files named by the BLAKE3 hash of their content.

use crate::{Error, Hash, Result};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

const ZSTD_LEVEL: i32 = 6;

struct HashingReader<R> {
    inner: R,
    hasher: blake3::Hasher,
    count: u64,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

/// Hashes `src` and writes its compressed blob into `store_root` unless the blob already exists.
/// Returns the content hash, the size, the compressed size and whether a new blob was written.
pub fn add_file(store_root: &Path, src: &Path) -> Result<(Hash, u64, u64, bool)> {
    let tmp_dir = store_root.join("blobs").join(".tmp");
    fs::create_dir_all(&tmp_dir)?;
    let tmp = tmp_dir.join(format!("{}-{}", std::process::id(), rand_suffix()));
    let mut reader = HashingReader {
        inner: BufReader::with_capacity(1 << 20, File::open(src)?),
        hasher: blake3::Hasher::new(),
        count: 0,
    };
    {
        let mut enc = zstd::Encoder::new(BufWriter::new(File::create(&tmp)?), ZSTD_LEVEL)?;
        io::copy(&mut reader, &mut enc)?;
        enc.finish()?.flush()?;
    }
    let hash = Hash::from_bytes(*reader.hasher.finalize().as_bytes());
    let dest = store_root.join(hash.blob_path());
    if dest.exists() {
        fs::remove_file(&tmp)?;
        return Ok((hash, reader.count, fs::metadata(&dest)?.len(), false));
    }
    let stored = fs::metadata(&tmp)?.len();
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::rename(&tmp, &dest)?;
    Ok((hash, reader.count, stored, true))
}

fn rand_suffix() -> u64 {
    use rand::RngCore;
    rand::rngs::OsRng.next_u64()
}

/// Decompresses the blob at `compressed` into `dest`, checking size and hash.
/// `dest` is removed again if verification fails.
pub fn decode_verified(compressed: &Path, dest: &Path, expected: &Hash, size: u64) -> Result<()> {
    let result = (|| -> Result<()> {
        let dec = zstd::Decoder::new(BufReader::new(File::open(compressed)?))?;
        // One byte beyond the declared size is enough to detect an oversized (decompression-bomb) blob.
        let mut limited = dec.take(size.saturating_add(1));
        let mut hashing = HashingReader { inner: &mut limited, hasher: blake3::Hasher::new(), count: 0 };
        let mut out = BufWriter::new(File::create(dest)?);
        io::copy(&mut hashing, &mut out)?;
        out.flush()?;
        if hashing.count != size {
            return Err(Error::CorruptBlob(expected.to_hex(), format!("size {} != {}", hashing.count, size)));
        }
        let got = Hash::from_bytes(*hashing.hasher.finalize().as_bytes());
        if &got != expected {
            return Err(Error::CorruptBlob(expected.to_hex(), format!("hash mismatch ({got})")));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_decode() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("f.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let store = dir.path().join("store");

        let (h, size, stored, new) = add_file(&store, &src).unwrap();
        assert!(new);
        assert_eq!(stored, fs::metadata(store.join(h.blob_path())).unwrap().len());
        assert_eq!(size, data.len() as u64);
        assert_eq!(h, Hash::of(&data));
        let (h2, _, _, new2) = add_file(&store, &src).unwrap();
        assert_eq!(h2, h);
        assert!(!new2);

        let out = dir.path().join("out.bin");
        decode_verified(&store.join(h.blob_path()), &out, &h, size).unwrap();
        assert_eq!(fs::read(&out).unwrap(), data);
    }

    #[test]
    fn rejects_wrong_hash_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("f");
        fs::write(&src, b"hello world").unwrap();
        let store = dir.path().join("s");
        let (h, size, _, _) = add_file(&store, &src).unwrap();
        let blob = store.join(h.blob_path());
        let out = dir.path().join("o");

        assert!(decode_verified(&blob, &out, &Hash::of(b"other"), size).is_err());
        assert!(!out.exists());
        assert!(decode_verified(&blob, &out, &h, size - 1).is_err());
        assert!(decode_verified(&blob, &out, &h, size + 1).is_err());
        assert!(!out.exists());
    }
}
