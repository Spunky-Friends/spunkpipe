//! Asks the user before a clicked link is allowed to set up a mod server.
//!
//! Mods are code, and any web page can open a `spunkpipe://` link, so a link must never join a
//! server without the person seeing where it points and saying yes. If no dialog can be shown,
//! the answer is "no": failing closed is the only safe choice.

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    /// No way to show a dialog on this machine.
    Unavailable,
}

/// Characters allowed in the host shown to the user. Anything else (spaces, quotes, non-ASCII
/// look-alikes) is refused outright, which also keeps the text safe to hand to a dialog program.
fn host_is_plain(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 200
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/' | '_' | '[' | ']'))
}

/// The dialog text, or `None` if the host is not plain ASCII.
pub fn prompt_text(host: &str, game: &str, fingerprint: &str, replacing: Option<&str>) -> Option<String> {
    if !host_is_plain(host) || !host_is_plain(game) || (replacing.is_some_and(|r| !host_is_plain(r))) {
        return None;
    }
    let mut t = String::new();
    if let Some(old) = replacing {
        t.push_str(&format!("WARNING: this replaces the server you currently use ({old}).\n\n"));
    }
    t.push_str(&format!(
        "A link wants SpunkPipe to download and install mods from:\n\n    {host}\n    Game: {game}\n    Key fingerprint: {fingerprint}\n\nMods can run programs on your computer. Only continue if the person who sent you this link told you to, and the fingerprint matches what they gave you."
    ));
    Some(t)
}

pub fn ask(text: &str) -> Answer {
    imp::ask(text)
}

/// The result of asking the user to pick a folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Picked {
    Folder(std::path::PathBuf),
    Cancelled,
    /// No way to show a folder chooser on this machine.
    Unavailable,
}

/// Shows the system's folder chooser, starting at `start` if given.
pub fn pick_folder(title: &str, start: Option<&std::path::Path>) -> Picked {
    imp::pick_folder(title, start)
}

/// Turns a chooser program's exit status and output into a result.
fn picked(success: bool, stdout: &[u8]) -> Picked {
    let s = String::from_utf8_lossy(stdout);
    let s = s.trim();
    if success && !s.is_empty() {
        Picked::Folder(std::path::PathBuf::from(s))
    } else {
        Picked::Cancelled
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::{Answer, Picked};
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn has(cmd: &str) -> bool {
        Command::new(cmd).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
    }

    pub fn pick_folder(title: &str, start: Option<&Path>) -> Picked {
        let start = start.map(|p| format!("{}/", p.display()));
        let out = if has("zenity") {
            let mut c = Command::new("zenity");
            c.args(["--file-selection", "--directory", "--title", title]);
            if let Some(s) = &start {
                c.args(["--filename", s]);
            }
            c.output()
        } else if has("kdialog") {
            Command::new("kdialog").args(["--title", title, "--getexistingdirectory", start.as_deref().unwrap_or(".")]).output()
        } else {
            return Picked::Unavailable;
        };
        match out {
            Ok(o) => super::picked(o.status.success(), &o.stdout),
            Err(_) => Picked::Unavailable,
        }
    }

    pub fn ask(text: &str) -> Answer {
        if has("zenity") {
            let ok = Command::new("zenity")
                .args(["--question", "--title", "SpunkPipe", "--width", "480", "--ok-label", "Install mods", "--cancel-label", "Cancel", "--default-cancel", "--no-markup", "--text", text])
                .status()
                .is_ok_and(|s| s.success());
            return if ok { Answer::Yes } else { Answer::No };
        }
        if has("kdialog") {
            let ok = Command::new("kdialog")
                .args(["--title", "SpunkPipe", "--yes-label", "Install mods", "--no-label", "Cancel", "--warningyesno", text])
                .status()
                .is_ok_and(|s| s.success());
            return if ok { Answer::Yes } else { Answer::No };
        }
        Answer::Unavailable
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Answer, Picked};
    use std::path::Path;
    use std::process::Command;

    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
    }

    pub fn pick_folder(title: &str, start: Option<&Path>) -> Picked {
        let default = start
            .filter(|p| p.is_dir())
            .map(|p| format!(" default location POSIX file \"{}\"", esc(&p.display().to_string())))
            .unwrap_or_default();
        let script = format!("POSIX path of (choose folder with prompt \"{}\"{default})", esc(title));
        match Command::new("osascript").args(["-e", &script]).output() {
            Ok(o) => super::picked(o.status.success(), &o.stdout),
            Err(_) => Picked::Unavailable,
        }
    }

    pub fn ask(text: &str) -> Answer {
        // The text only ever contains plain ASCII from `prompt_text`, but escape anyway.
        let esc = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let script = format!(
            "display dialog \"{esc}\" with title \"SpunkPipe\" buttons {{\"Cancel\", \"Install mods\"}} default button \"Cancel\" cancel button \"Cancel\" with icon caution"
        );
        match Command::new("osascript").args(["-e", &script]).output() {
            Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Install mods") => Answer::Yes,
            Ok(_) => Answer::No,
            Err(_) => Answer::Unavailable,
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::{Answer, Picked};
    use std::path::Path;
    use std::process::Command;

    pub fn pick_folder(title: &str, start: Option<&Path>) -> Picked {
        // Title and start folder go through environment variables so they are never parsed as code.
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
            $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
            $d.Description = $env:SPUNKPIPE_TITLE; \
            if ($env:SPUNKPIPE_START) { $d.SelectedPath = $env:SPUNKPIPE_START }; \
            if ($d.ShowDialog() -eq 'OK') { [Console]::Out.Write($d.SelectedPath); exit 0 } else { exit 1 }";
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Sta", "-Command", script])
            .env("SPUNKPIPE_TITLE", title)
            .env("SPUNKPIPE_START", start.map(|p| p.display().to_string()).unwrap_or_default())
            .output();
        match out {
            Ok(o) => super::picked(o.status.success(), &o.stdout),
            Err(_) => Picked::Unavailable,
        }
    }

    pub fn ask(text: &str) -> Answer {
        // The text goes through an environment variable so it is never parsed as code.
        let script = "Add-Type -AssemblyName PresentationFramework; \
            $r = [System.Windows.MessageBox]::Show($env:SPUNKPIPE_PROMPT, 'SpunkPipe', 'YesNo', 'Warning', 'No'); \
            if ($r -eq 'Yes') { exit 0 } else { exit 1 }";
        match Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).env("SPUNKPIPE_PROMPT", format!("{text}\n\nInstall mods from this server?")).status() {
            Ok(s) if s.success() => Answer::Yes,
            Ok(_) => Answer::No,
            Err(_) => Answer::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_host_and_fingerprint() {
        let t = prompt_text("mods.example.com", "7dtd", "a1b2-c3d4-e5f6-0718", None).unwrap();
        assert!(t.contains("mods.example.com") && t.contains("a1b2-c3d4-e5f6-0718") && t.contains("Game: 7dtd"));
        assert!(!t.contains("WARNING"));
    }

    #[test]
    fn replacing_an_existing_server_is_called_out() {
        let t = prompt_text("evil.example", "7dtd", "0000-0000-0000-0000", Some("mods.spunky.boats")).unwrap();
        assert!(t.starts_with("WARNING"));
        assert!(t.contains("mods.spunky.boats"));
    }

    #[test]
    fn a_chosen_folder_comes_back_trimmed_and_a_cancel_is_a_cancel() {
        assert_eq!(picked(true, b"/data/Mods\n"), Picked::Folder("/data/Mods".into()));
        assert_eq!(picked(true, b"  \n"), Picked::Cancelled);
        assert_eq!(picked(false, b"/data/Mods\n"), Picked::Cancelled);
    }

    #[test]
    fn odd_hosts_are_refused() {
        for h in ["exa mple.com", "mods.exämple.com", "a\"b", "a';b", "<b>x</b>", "", "x\ny"] {
            assert!(prompt_text(h, "7dtd", "0000-0000-0000-0000", None).is_none(), "{h:?}");
        }
    }
}
