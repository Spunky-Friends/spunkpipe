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
