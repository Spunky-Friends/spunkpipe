//! What the tray shows, as plain data. Each OS shell just draws it, so this part is testable.

use spunkpipe_daemon::{Mode, Snapshot, Status};

#[derive(Clone, Debug)]
pub struct GameView {
    pub game: String,
    /// Human name for headings.
    pub name: String,
    pub snapshot: Snapshot,
    /// 0..=100, meaningful while downloading.
    pub percent: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CheckNow(String),
    Update(String),
    SetMode(String, Mode),
    OpenFolder(String),
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Label(String),
    Separator,
    Action { label: String, enabled: bool, action: Action },
    Choice { label: String, checked: bool, action: Action },
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub fn status_text(v: &GameView) -> String {
    match &v.snapshot.status {
        Status::Starting => "Starting…".into(),
        Status::Checking => "Checking for updates…".into(),
        Status::Idle { serial } => format!("Up to date (version {serial})"),
        Status::UpdateAvailable { serial, files, bytes } => {
            format!("Update available: version {serial}, {files} files, {}", human_bytes(*bytes))
        }
        Status::Downloading { serial } => format!("Downloading version {serial}… {}%", v.percent),
        Status::WaitingForGame { serial } => format!("Version {serial} ready; will install when the game closes"),
        Status::Applying { serial } => format!("Installing version {serial}…"),
        Status::Error(e) => format!("Problem: {e}"),
    }
}

/// Only the Linux tray changes its icon state; the other platforms just show the tooltip.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn needs_attention(views: &[GameView]) -> bool {
    views.iter().any(|v| matches!(v.snapshot.status, Status::UpdateAvailable { .. } | Status::Error(_)))
}

pub fn tooltip(views: &[GameView]) -> String {
    views.iter().map(|v| format!("{}: {}", v.name, status_text(v))).collect::<Vec<_>>().join("\n")
}

pub fn build_menu(views: &[GameView]) -> Vec<Item> {
    let mut items = Vec::new();
    for v in views {
        let g = &v.game;
        let status = &v.snapshot.status;
        items.push(Item::Label(v.name.clone()));
        items.push(Item::Label(status_text(v)));
        items.push(Item::Action {
            label: "Check now".into(),
            enabled: !matches!(status, Status::Checking | Status::Downloading { .. } | Status::Applying { .. }),
            action: Action::CheckNow(g.clone()),
        });
        let update_label = match status {
            Status::UpdateAvailable { bytes, .. } => format!("Download and install update ({})", human_bytes(*bytes)),
            _ => "Download and install update".into(),
        };
        items.push(Item::Action {
            label: update_label,
            enabled: matches!(status, Status::UpdateAvailable { .. }),
            action: Action::Update(g.clone()),
        });
        for (mode, label) in [
            (Mode::Auto, "Automatic updates"),
            (Mode::Manual, "Ask me before downloading"),
            (Mode::Paused, "Paused"),
        ] {
            items.push(Item::Choice {
                label: label.into(),
                checked: v.snapshot.mode == mode,
                action: Action::SetMode(g.clone(), mode),
            });
        }
        items.push(Item::Action { label: "Open mods folder".into(), enabled: true, action: Action::OpenFolder(g.clone()) });
        items.push(Item::Separator);
    }
    items.push(Item::Action { label: "Quit SpunkPipe".into(), enabled: true, action: Action::Quit });
    items
}

/// A desktop notification worth showing for this change, if any: (title, body).
pub fn notification_for(prev: Option<&Status>, v: &GameView) -> Option<(String, String)> {
    let new = &v.snapshot.status;
    if prev == Some(new) {
        return None;
    }
    let title = format!("SpunkPipe: {}", v.name);
    match (prev, new) {
        (_, Status::UpdateAvailable { .. }) if v.snapshot.mode != Mode::Auto => {
            Some((title, format!("{}. Open the tray menu to download it.", status_text(v))))
        }
        (_, Status::WaitingForGame { serial }) => {
            Some((title, format!("Mods version {serial} downloaded. It will install when you close the game.")))
        }
        (Some(Status::Applying { .. }), Status::Idle { serial }) => Some((title, format!("Mods updated to version {serial}."))),
        (Some(Status::Error(_)), Status::Error(_)) => None,
        (_, Status::Error(e)) => Some((title, format!("Update problem: {e}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(mode: Mode, status: Status) -> GameView {
        GameView { game: "7dtd".into(), name: "7 Days to Die".into(), snapshot: Snapshot { mode, status }, percent: 42 }
    }

    fn find<'a>(items: &'a [Item], label: &str) -> &'a Item {
        items
            .iter()
            .find(|i| matches!(i, Item::Action { label: l, .. } | Item::Choice { label: l, .. } if l.starts_with(label)))
            .unwrap_or_else(|| panic!("no item {label}"))
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn update_button_only_when_an_update_is_waiting() {
        let m = build_menu(&[view(Mode::Manual, Status::Idle { serial: 3 })]);
        assert!(matches!(find(&m, "Download and install"), Item::Action { enabled: false, .. }));
        let m = build_menu(&[view(Mode::Manual, Status::UpdateAvailable { serial: 4, files: 2, bytes: 3 * 1024 * 1024 })]);
        let Item::Action { enabled, label, action } = find(&m, "Download and install") else { unreachable!() };
        assert!(*enabled);
        assert!(label.contains("3.0 MB"));
        assert_eq!(*action, Action::Update("7dtd".into()));
    }

    #[test]
    fn exactly_the_current_mode_is_checked() {
        let m = build_menu(&[view(Mode::Paused, Status::Idle { serial: 1 })]);
        let checked: Vec<_> = m.iter().filter_map(|i| match i { Item::Choice { label, checked: true, .. } => Some(label.as_str()), _ => None }).collect();
        assert_eq!(checked, ["Paused"]);
    }

    #[test]
    fn check_now_is_off_while_busy_and_quit_is_last() {
        let m = build_menu(&[view(Mode::Auto, Status::Downloading { serial: 2 })]);
        assert!(matches!(find(&m, "Check now"), Item::Action { enabled: false, .. }));
        assert!(matches!(m.last(), Some(Item::Action { action: Action::Quit, .. })));
        assert!(tooltip(&[view(Mode::Auto, Status::Downloading { serial: 2 })]).contains("42%"));
    }

    #[test]
    fn notifications_are_sparse() {
        let idle = Status::Idle { serial: 1 };
        // Auto mode already handles updates, so announcing one would be noise.
        let auto = view(Mode::Auto, Status::UpdateAvailable { serial: 2, files: 1, bytes: 10 });
        assert!(notification_for(Some(&idle), &auto).is_none());
        let manual = view(Mode::Manual, Status::UpdateAvailable { serial: 2, files: 1, bytes: 10 });
        assert!(notification_for(Some(&idle), &manual).is_some());
        assert!(notification_for(Some(&manual.snapshot.status), &manual).is_none());
        let done = view(Mode::Auto, Status::Idle { serial: 2 });
        assert!(notification_for(Some(&Status::Applying { serial: 2 }), &done).is_some());
        assert!(notification_for(Some(&Status::Checking), &done).is_none(), "plain no-op checks stay silent");
        let err = view(Mode::Auto, Status::Error("boom".into()));
        assert!(notification_for(Some(&Status::Checking), &err).is_some());
        assert!(notification_for(Some(&Status::Error("boom".into())), &view(Mode::Auto, Status::Error("boom2".into()))).is_none());
    }
}
