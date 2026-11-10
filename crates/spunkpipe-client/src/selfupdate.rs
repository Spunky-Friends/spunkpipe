//! `spunkpipe update`: replaces the installed programs with a newer signed release.
//!
//! The trust root is the release public key built into the program (`release-key.pub`), not the
//! web host: the host serves a signed `release.json` listing the SHA-256 of every archive.

use crate::source::{read_all, Source};
use spunkpipe_core::release::{parse_version, sha256_hex, ReleaseManifest, RELEASE_FILE, RELEASE_SIG_FILE};
use spunkpipe_core::{Error, PublicKey, Result};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

/// The release signing key, set at build time from the file `release-key.pub` in the repository.
pub const BUILT_IN_RELEASE_KEY: &str = include_str!("../../../release-key.pub");
pub const DEFAULT_BASE_URL: &str = "https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const PROGRAMS: [&str; 2] = ["spunkpipe", "spunkpipe-tray"];
const MAX_ARCHIVE: u64 = 300 << 20;

/// The release target this build belongs to, if we publish one.
pub fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

pub fn asset_name(version: &str, target: &str) -> String {
    let ext = if target.contains("windows") { "zip" } else { "tar.gz" };
    format!("spunkpipe-{version}-{target}.{ext}")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate { version: String },
    /// A newer version exists (only reported when `check_only` is set).
    Available { version: String },
    Updated { from: String, to: String },
}

pub struct UpdateOptions {
    pub key: PublicKey,
    pub current_version: String,
    pub target: String,
    /// Where the installed programs live.
    pub install_dir: PathBuf,
    pub check_only: bool,
}

/// Fetches and verifies the signed release manifest.
pub fn fetch_release(source: &dyn Source, key: &PublicKey) -> Result<ReleaseManifest> {
    let bytes = read_all(source, RELEASE_FILE, 1 << 20)?;
    let sig = read_all(source, RELEASE_SIG_FILE, 1024)?;
    key.verify(&bytes, &String::from_utf8_lossy(&sig))?;
    ReleaseManifest::from_bytes(&bytes)
}

pub fn update(source: &dyn Source, opts: &UpdateOptions) -> Result<Outcome> {
    let rel = fetch_release(source, &opts.key)?;
    // Only ever move forward: a replayed older signed release must not downgrade anyone.
    if parse_version(&rel.version)? <= parse_version(&opts.current_version)? {
        return Ok(Outcome::UpToDate { version: opts.current_version.clone() });
    }
    if opts.check_only {
        return Ok(Outcome::Available { version: rel.version });
    }
    let name = asset_name(&rel.version, &opts.target);
    let asset = rel
        .asset(&name)
        .ok_or_else(|| Error::Other(format!("release {} has no build for {}", rel.version, opts.target)))?;
    if asset.size > MAX_ARCHIVE {
        return Err(Error::Other("release archive is unreasonably large".into()));
    }
    let mut data = Vec::new();
    source.open(&name, 0)?.take(asset.size + 1).read_to_end(&mut data)?;
    if data.len() as u64 != asset.size {
        return Err(Error::Other(format!("{name}: size does not match the signed manifest")));
    }
    if sha256_hex(&data[..])? != asset.sha256 {
        return Err(Error::Other(format!("{name}: checksum does not match the signed manifest")));
    }
    install_archive(&data, &name, &opts.install_dir)?;
    Ok(Outcome::Updated { from: opts.current_version.clone(), to: rel.version })
}

/// Unpacks `spunkpipe` and `spunkpipe-tray` from the archive into `dir`, replacing the old ones.
/// Nothing else in the archive is ever written, whatever paths it contains.
pub fn install_archive(data: &[u8], name: &str, dir: &Path) -> Result<()> {
    let files = if name.ends_with(".zip") { read_zip(data)? } else { read_tar_gz(data)? };
    if files.is_empty() {
        return Err(Error::Other("archive does not contain the programs".into()));
    }
    fs::create_dir_all(dir)?;
    // Stage every file first so a bad archive cannot leave a half-updated install.
    let mut staged = Vec::new();
    for (n, bytes) in &files {
        let new = dir.join(format!("{n}.new"));
        fs::write(&new, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
        }
        staged.push((dir.join(n), new));
    }
    for (dest, new) in staged {
        replace(&dest, &new)?;
    }
    Ok(())
}

fn is_wanted(file_name: &str) -> bool {
    PROGRAMS.iter().any(|p| file_name == *p || file_name == format!("{p}.exe"))
}

/// Swaps in the new file. The old one is renamed aside first: that works even while it runs
/// (including on Windows, where a running program cannot be overwritten but can be renamed).
fn replace(dest: &Path, new: &Path) -> Result<()> {
    let old = dest.with_extension("old");
    let _ = fs::remove_file(&old);
    if dest.exists() {
        fs::rename(dest, &old)?;
    }
    if let Err(e) = fs::rename(new, dest) {
        let _ = fs::rename(&old, dest); // put the original back
        return Err(e.into());
    }
    let _ = fs::remove_file(&old); // may fail on Windows while it runs; cleaned up next time
    Ok(())
}

type Files = Vec<(String, Vec<u8>)>;

fn read_tar_gz(data: &[u8]) -> Result<Files> {
    let mut out = Vec::new();
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(data)));
    for e in ar.entries()? {
        let mut e = e?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let path = e.path()?.into_owned();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if is_wanted(&name) {
            let mut b = Vec::new();
            e.take(MAX_ARCHIVE).read_to_end(&mut b)?;
            out.push((name, b));
        }
    }
    Ok(out)
}

fn read_zip(data: &[u8]) -> Result<Files> {
    let mut out = Vec::new();
    let mut z = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| Error::Other(e.to_string()))?;
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|e| Error::Other(e.to_string()))?;
        if !f.is_file() {
            continue;
        }
        let Ok(zip_name) = f.name() else { continue };
        let Some(name) = Path::new(&*zip_name).file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if is_wanted(&name) {
            let mut b = Vec::new();
            io::Read::read_to_end(&mut f.take(MAX_ARCHIVE), &mut b)?;
            out.push((name, b));
        }
    }
    Ok(out)
}

/// The key to trust for updates, or an explanation of why there is none.
pub fn release_key() -> Result<PublicKey> {
    let k = BUILT_IN_RELEASE_KEY.trim();
    if k.is_empty() {
        return Err(Error::Other(
            "this build has no release signing key, so it cannot verify updates (set release-key.pub when building releases)".into(),
        ));
    }
    PublicKey::from_base64(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::DirSource;
    use spunkpipe_core::release::ReleaseManifest;
    use spunkpipe_core::SecretKey;
    use std::io::Write;

    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (n, d) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(d.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, n, *d).unwrap();
        }
        let raw = b.into_inner().unwrap();
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&raw).unwrap();
        e.finish().unwrap()
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn publish(dir: &Path, key: &SecretKey, version: &str, target: &str, archive: &[u8]) {
        fs::write(dir.join(asset_name(version, target)), archive).unwrap();
        let m = ReleaseManifest::from_dir(dir, version).unwrap();
        let bytes = m.to_bytes().unwrap();
        fs::write(dir.join(RELEASE_FILE), &bytes).unwrap();
        fs::write(dir.join(RELEASE_SIG_FILE), key.sign(&bytes)).unwrap();
    }

    fn opts(key: &SecretKey, install: &Path, current: &str, check_only: bool) -> UpdateOptions {
        UpdateOptions {
            key: key.public(),
            current_version: current.into(),
            target: "x86_64-unknown-linux-musl".into(),
            install_dir: install.into(),
            check_only,
        }
    }

    #[test]
    fn updates_to_a_newer_signed_release() {
        let d = tempfile::tempdir().unwrap();
        let (web, install) = (d.path().join("web"), d.path().join("bin"));
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("spunkpipe"), b"old cli").unwrap();
        let key = SecretKey::generate();
        let a = tar_gz(&[("spunkpipe", b"new cli"), ("spunkpipe-tray", b"new tray"), ("README.md", b"x")]);
        publish(&web, &key, "0.2.0", "x86_64-unknown-linux-musl", &a);
        let src = DirSource(web);

        assert_eq!(update(&src, &opts(&key, &install, "0.1.0", true)).unwrap(), Outcome::Available { version: "0.2.0".into() });
        assert_eq!(fs::read(install.join("spunkpipe")).unwrap(), b"old cli", "check_only changes nothing");

        let out = update(&src, &opts(&key, &install, "0.1.0", false)).unwrap();
        assert_eq!(out, Outcome::Updated { from: "0.1.0".into(), to: "0.2.0".into() });
        assert_eq!(fs::read(install.join("spunkpipe")).unwrap(), b"new cli");
        assert_eq!(fs::read(install.join("spunkpipe-tray")).unwrap(), b"new tray");
        assert!(!install.join("README.md").exists(), "only the programs are installed");
        assert!(!install.join("spunkpipe.old").exists());

        assert_eq!(update(&src, &opts(&key, &install, "0.2.0", false)).unwrap(), Outcome::UpToDate { version: "0.2.0".into() });
        assert_eq!(update(&src, &opts(&key, &install, "0.3.0", false)).unwrap(), Outcome::UpToDate { version: "0.3.0".into() }, "never downgrades");
    }

    #[test]
    fn rejects_wrong_key_tampered_archive_and_bad_checksum() {
        let d = tempfile::tempdir().unwrap();
        let (web, install) = (d.path().join("web"), d.path().join("bin"));
        fs::create_dir_all(&web).unwrap();
        let key = SecretKey::generate();
        let a = tar_gz(&[("spunkpipe", b"new")]);
        publish(&web, &key, "0.2.0", "x86_64-unknown-linux-musl", &a);
        let src = DirSource(web.clone());

        let other = SecretKey::generate();
        assert!(matches!(update(&src, &opts(&other, &install, "0.1.0", false)), Err(Error::BadSignature)));

        // The archive is swapped for a different one of the same size.
        let name = asset_name("0.2.0", "x86_64-unknown-linux-musl");
        let mut evil = a.clone();
        let last = evil.len() - 1;
        evil[last] ^= 0xff;
        fs::write(web.join(&name), &evil).unwrap();
        let err = update(&src, &opts(&key, &install, "0.1.0", false)).unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
        assert!(!install.join("spunkpipe").exists());
    }

    #[test]
    fn unpacks_zip_and_ignores_path_tricks() {
        let d = tempfile::tempdir().unwrap();
        let z = zip_of(&[("spunkpipe.exe", b"cli"), ("spunkpipe-tray.exe", b"tray"), ("../../evil.exe", b"x"), ("dir/other.dll", b"y")]);
        install_archive(&z, "spunkpipe-0.2.0-x86_64-pc-windows-msvc.zip", &d.path().join("bin")).unwrap();
        let mut names: Vec<_> = fs::read_dir(d.path().join("bin")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["spunkpipe-tray.exe", "spunkpipe.exe"]);
        assert!(!d.path().join("evil.exe").exists());

        assert!(install_archive(&tar_gz(&[("other", b"x")]), "x.tar.gz", &d.path().join("b2")).is_err());
    }

    #[test]
    fn built_in_key_is_optional_but_explained() {
        // The checked-in key file may be empty until a release key is generated.
        match release_key() {
            Ok(_) => {}
            Err(e) => assert!(e.to_string().contains("release-key.pub")),
        }
    }
}
