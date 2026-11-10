//! Start the tray app at login, using each OS's own mechanism and no extra dependencies.

use spunkpipe_core::{Error, Result};
use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(not(windows))]
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Other("cannot find the home folder".into()))
}

/// XDG autostart entry (GNOME, KDE, XFCE and others).
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=SpunkPipe\nComment=Keeps your game mods in sync\nExec=\"{}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    )
}

pub const MAC_LABEL: &str = "boats.spunky.spunkpipe";

pub fn launch_agent(exe: &Path) -> String {
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>{MAC_LABEL}</string>\n  <key>ProgramArguments</key><array><string>{exe}</string></array>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><false/>\n</dict>\n</plist>\n"
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn entry_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map_or_else(|| home().map(|h| h.join(".config")), Ok)?;
    Ok(base.join("autostart/spunkpipe.desktop"))
}

#[cfg(target_os = "macos")]
fn entry_path() -> Result<PathBuf> {
    Ok(home()?.join(format!("Library/LaunchAgents/{MAC_LABEL}.plist")))
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn enable(exe: &Path) -> Result<()> {
    let out = std::process::Command::new("reg")
        .args(["add", RUN_KEY, "/v", "SpunkPipe", "/t", "REG_SZ", "/f", "/d"])
        .arg(format!("\"{}\"", exe.display()))
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!("reg add failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

#[cfg(windows)]
pub fn disable() -> Result<()> {
    // Deleting a value that is not there is fine.
    let _ = std::process::Command::new("reg").args(["delete", RUN_KEY, "/v", "SpunkPipe", "/f"]).output()?;
    Ok(())
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", "SpunkPipe"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn enable(exe: &Path) -> Result<()> {
    let path = entry_path()?;
    let body = if cfg!(target_os = "macos") { launch_agent(exe) } else { desktop_entry(exe) };
    spunkpipe_core::fsutil::atomic_write(&path, body.as_bytes())
}

#[cfg(not(windows))]
pub fn disable() -> Result<()> {
    match std::fs::remove_file(entry_path()?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    entry_path().map(|p| p.exists()).unwrap_or(false)
}

// ---- spunkpipe:// link handler ----

/// Desktop entry that makes `spunkpipe://` links open the tray app (Linux).
pub fn handler_desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=SpunkPipe\nComment=Join a mod server\nExec=\"{}\" %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/spunkpipe;\n",
        exe.display()
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn handler_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map_or_else(|| home().map(|h| h.join(".local/share")), Ok)?;
    Ok(base.join("applications/spunkpipe-handler.desktop"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn register_url_handler(exe: &Path) -> Result<()> {
    let path = handler_path()?;
    spunkpipe_core::fsutil::atomic_write(&path, handler_desktop_entry(exe).as_bytes())?;
    // Best effort: tell the desktop which program handles the scheme.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", "spunkpipe-handler.desktop", "x-scheme-handler/spunkpipe"])
        .status();
    if let Some(dir) = path.parent() {
        let _ = std::process::Command::new("update-desktop-database").arg(dir).status();
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn unregister_url_handler() -> Result<()> {
    match std::fs::remove_file(handler_path()?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn url_handler_registered() -> bool {
    handler_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(windows)]
const CLASS_KEY: &str = r"HKCU\Software\Classes\spunkpipe";

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("reg").args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!("reg failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

#[cfg(windows)]
pub fn register_url_handler(exe: &Path) -> Result<()> {
    reg(&["add", CLASS_KEY, "/ve", "/d", "URL:SpunkPipe", "/f"])?;
    reg(&["add", CLASS_KEY, "/v", "URL Protocol", "/d", "", "/f"])?;
    let cmd = format!("\"{}\" \"%1\"", exe.display());
    reg(&["add", &format!(r"{CLASS_KEY}\shell\open\command"), "/ve", "/d", &cmd, "/f"])
}

#[cfg(windows)]
pub fn unregister_url_handler() -> Result<()> {
    let _ = std::process::Command::new("reg").args(["delete", CLASS_KEY, "/f"]).output()?;
    Ok(())
}

#[cfg(windows)]
pub fn url_handler_registered() -> bool {
    std::process::Command::new("reg").args(["query", CLASS_KEY]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// macOS registers the scheme through the app bundle (`SpunkPipe.app` next to the programs),
/// whose Info.plist declares it. Launch Services is told about the bundle here.
#[cfg(target_os = "macos")]
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let b = exe.parent()?.join("SpunkPipe.app");
    b.is_dir().then_some(b)
}

#[cfg(target_os = "macos")]
pub fn register_url_handler(exe: &Path) -> Result<()> {
    let bundle = app_bundle(exe).ok_or_else(|| Error::Other("SpunkPipe.app not found next to the programs".into()))?;
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let st = std::process::Command::new(lsregister).arg("-f").arg(&bundle).status()?;
    if st.success() {
        Ok(())
    } else {
        Err(Error::Other("lsregister failed".into()))
    }
}

#[cfg(target_os = "macos")]
pub fn unregister_url_handler() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn url_handler_registered() -> bool {
    std::env::current_exe().ok().and_then(|e| app_bundle(&e)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_entries_name_the_program() {
        let exe = Path::new("/opt/spunk pipe/spunkpipe-tray");
        assert!(desktop_entry(exe).contains("Exec=\"/opt/spunk pipe/spunkpipe-tray\""));
        let plist = launch_agent(Path::new("/a&b/tray"));
        assert!(plist.contains("<string>/a&amp;b/tray</string>"));
        assert!(plist.contains("RunAtLoad"));
    }

    #[test]
    fn handler_entry_declares_the_scheme() {
        let e = handler_desktop_entry(Path::new("/x/spunkpipe-tray"));
        assert!(e.contains("x-scheme-handler/spunkpipe"));
        assert!(e.contains("Exec=\"/x/spunkpipe-tray\" %u"));
    }

    #[cfg(unix)]
    #[test]
    fn enable_and_disable_on_unix() {
        let d = tempfile::tempdir().unwrap();
        // Point the platform's autostart location into a temp home.
        std::env::set_var("HOME", d.path());
        std::env::set_var("XDG_CONFIG_HOME", d.path().join("cfg"));
        assert!(!is_enabled());
        enable(Path::new("/x/spunkpipe-tray")).unwrap();
        assert!(is_enabled());
        disable().unwrap();
        assert!(!is_enabled());
        disable().unwrap();
    }
}
