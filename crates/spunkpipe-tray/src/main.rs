//! The SpunkPipe tray app: a small icon that keeps game mods up to date.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod icon;
mod menu;

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(any(windows, target_os = "macos"))]
mod desktop;

fn main() {
    if let Err(e) = run() {
        // With no console on Windows this is the only way the user hears about it.
        app::notify("SpunkPipe", &format!("{e:#}"));
        eprintln!("spunkpipe-tray: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(any(windows, target_os = "macos"))]
fn run() -> anyhow::Result<()> {
    desktop::run()
}
