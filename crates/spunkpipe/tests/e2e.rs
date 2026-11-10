use spunkpipe_client::{sync, DirSource, Report, SyncOptions};
use spunkpipe_core::{manifest_path, Error, SecretKey};
use spunkpipe_server::{publish, PublishOptions};
use std::fs;
use std::path::{Path, PathBuf};

struct Env {
    _d: tempfile::TempDir,
    src: PathBuf,
    web: PathBuf,
    target: PathBuf,
    state: PathBuf,
    key: String,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().to_path_buf();
    let e = Env {
        src: r.join("server/Mods"),
        web: r.join("web"),
        target: r.join("client/Mods"),
        state: r.join("client-state"),
        key: SecretKey::generate().to_base64(),
        _d: d,
    };
    fs::create_dir_all(&e.src).unwrap();
    fs::create_dir_all(&e.target).unwrap();
    e
}

fn put(root: &Path, rel: &str, data: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn pubkey(e: &Env) -> spunkpipe_core::PublicKey {
    SecretKey::from_base64(&e.key).unwrap().public()
}

fn do_publish(e: &Env) -> u64 {
    publish(&PublishOptions {
        source: e.src.clone(),
        out: e.web.clone(),
        game: "7dtd".into(),
        key: SecretKey::from_base64(&e.key).unwrap(),
        force: false,
        keep: 10,
        threads: 4,
    })
    .unwrap()
    .serial
}

fn opts(e: &Env) -> SyncOptions {
    SyncOptions {
        target: e.target.clone(),
        state: e.state.clone(),
        game: "7dtd".into(),
        serial: None,
        verify: false,
        dry_run: false,
        threads: 4,
    }
}

fn do_sync(e: &Env) -> Report {
    sync(&DirSource(e.web.clone()), &pubkey(e), &opts(e)).unwrap()
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn full_then_incremental_sync() {
    let e = env();
    put(&e.src, "CurryMod/ModInfo.xml", "<info v='1'/>");
    put(&e.src, "CurryMod/Config/blocks.xml", "<blocks/>");
    put(&e.src, "Other/Mod.dll", "binary one");
    do_publish(&e);

    let r = do_sync(&e);
    assert_eq!((r.placed, r.fetched_blobs, r.removed), (3, 3, 0));
    assert_eq!(read(&e.target, "CurryMod/ModInfo.xml"), "<info v='1'/>");

    // Nothing changed on the server: no work at all.
    assert!(do_sync(&e).up_to_date);

    // One file changes, one is added, one is removed: only the delta is fetched.
    put(&e.src, "CurryMod/ModInfo.xml", "<info v='2'/>");
    put(&e.src, "New/Thing.xml", "<new/>");
    fs::remove_file(e.src.join("Other/Mod.dll")).unwrap();
    do_publish(&e);
    let r = do_sync(&e);
    assert_eq!((r.fetched_blobs, r.placed, r.removed, r.unchanged), (2, 2, 1, 1));
    assert_eq!(read(&e.target, "CurryMod/ModInfo.xml"), "<info v='2'/>");
    assert!(!e.target.join("Other").exists(), "empty folder pruned");
    assert_eq!(r.backed_up, 0);
}

#[test]
fn users_own_files_are_left_alone_and_collisions_backed_up() {
    let e = env();
    put(&e.target, "MyOwnMod/ModInfo.xml", "mine");
    put(&e.target, "Shared/Same.xml", "the user's version");
    put(&e.src, "Shared/Same.xml", "the server's version");
    put(&e.src, "Server/A.xml", "a");
    do_publish(&e);

    let r = do_sync(&e);
    assert_eq!(r.backed_up, 1);
    assert_eq!(read(&e.target, "Shared/Same.xml"), "the server's version");
    assert_eq!(read(&e.target, "MyOwnMod/ModInfo.xml"), "mine");

    // Removing a mod from the server must not delete the user's own mod.
    fs::remove_dir_all(e.src.join("Server")).unwrap();
    do_publish(&e);
    do_sync(&e);
    assert!(!e.target.join("Server").exists());
    assert_eq!(read(&e.target, "MyOwnMod/ModInfo.xml"), "mine");
}

#[test]
fn locally_modified_managed_file_is_backed_up_on_removal() {
    let e = env();
    put(&e.src, "M/a.xml", "orig");
    put(&e.src, "M/b.xml", "keep");
    do_publish(&e);
    do_sync(&e);
    put(&e.target, "M/a.xml", "user edited this");
    fs::remove_file(e.src.join("M/a.xml")).unwrap();
    do_publish(&e);
    let r = do_sync(&e);
    assert_eq!((r.removed, r.backed_up), (1, 1));
    assert!(!e.target.join("M/a.xml").exists());
}

#[test]
fn verify_repairs_deleted_files() {
    let e = env();
    put(&e.src, "M/a.xml", "orig");
    do_publish(&e);
    do_sync(&e);
    fs::remove_file(e.target.join("M/a.xml")).unwrap();
    assert!(do_sync(&e).up_to_date);
    let r = sync(&DirSource(e.web.clone()), &pubkey(&e), &SyncOptions { verify: true, ..opts(&e) }).unwrap();
    assert_eq!(r.placed, 1);
    assert_eq!(read(&e.target, "M/a.xml"), "orig");
}

#[test]
fn rollback_is_explicit_only() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    let s1 = do_publish(&e);
    do_sync(&e);
    put(&e.src, "M/a.xml", "two");
    do_publish(&e);
    do_sync(&e);
    assert_eq!(read(&e.target, "M/a.xml"), "two");

    let r = sync(&DirSource(e.web.clone()), &pubkey(&e), &SyncOptions { serial: Some(s1), ..opts(&e) }).unwrap();
    assert_eq!(r.serial, s1);
    assert_eq!(read(&e.target, "M/a.xml"), "one");
}

#[test]
fn refuses_replayed_old_server_state() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    do_publish(&e);
    put(&e.src, "M/a.xml", "two");
    do_publish(&e);
    do_sync(&e);
    // A hostile host rewinds the pointer to the old (validly signed) manifest.
    fs::write(e.web.join("latest"), "1\n").unwrap();
    let err = sync(&DirSource(e.web.clone()), &pubkey(&e), &opts(&e)).unwrap_err();
    assert!(err.to_string().contains("backwards"), "{err}");
    assert_eq!(read(&e.target, "M/a.xml"), "two");
}

#[test]
fn rejects_wrong_key_and_tampering() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    let s = do_publish(&e);

    let other = SecretKey::generate().public();
    let err = sync(&DirSource(e.web.clone()), &other, &opts(&e)).unwrap_err();
    assert!(matches!(err, Error::BadSignature), "{err}");

    let mp = e.web.join(manifest_path(s));
    let tampered = fs::read_to_string(&mp).unwrap().replace("\"size\": 3", "\"size\": 4");
    fs::write(&mp, tampered).unwrap();
    let err = sync(&DirSource(e.web.clone()), &pubkey(&e), &opts(&e)).unwrap_err();
    assert!(matches!(err, Error::BadSignature), "{err}");
    assert!(!e.target.join("M/a.xml").exists());
}

#[test]
fn corrupt_blob_is_rejected_and_nothing_is_installed() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    put(&e.src, "M/b.xml", "two");
    do_publish(&e);
    let blob = spunkpipe_core::Hash::of(b"two").blob_path();
    fs::write(e.web.join(blob), b"garbage").unwrap();
    assert!(sync(&DirSource(e.web.clone()), &pubkey(&e), &opts(&e)).is_err());
    assert!(!e.target.join("M/a.xml").exists(), "apply must not start until all blobs verify");
    assert!(!e.state.join("lock.json").exists());
}

#[test]
fn dry_run_changes_nothing() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    do_publish(&e);
    let r = sync(&DirSource(e.web.clone()), &pubkey(&e), &SyncOptions { dry_run: true, ..opts(&e) }).unwrap();
    assert_eq!((r.placed, r.fetched_blobs), (1, 1));
    assert!(!e.target.join("M/a.xml").exists());
    assert!(!e.state.exists());
}

#[test]
fn identical_files_share_one_download() {
    let e = env();
    put(&e.src, "A/same.xml", "dup");
    put(&e.src, "B/same.xml", "dup");
    do_publish(&e);
    let r = do_sync(&e);
    assert_eq!((r.placed, r.fetched_blobs), (2, 1));
    assert_eq!(read(&e.target, "A/same.xml"), "dup");
    assert_eq!(read(&e.target, "B/same.xml"), "dup");
}

#[test]
fn state_inside_target_is_refused() {
    let e = env();
    put(&e.src, "M/a.xml", "one");
    do_publish(&e);
    let bad = SyncOptions { state: e.target.join(".state"), ..opts(&e) };
    assert!(sync(&DirSource(e.web.clone()), &pubkey(&e), &bad).is_err());
}
