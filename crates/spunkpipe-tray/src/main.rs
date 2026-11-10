//! The SpunkPipe tray app: a small icon that keeps game mods up to date.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod confirm;
mod icon;
mod link;
mod menu;
mod single;

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(any(windows, target_os = "macos"))]
mod desktop;

fn main() {
    if let Err(e) = start() {
        // With no console on Windows this is the only way the user hears about it.
        app::notify("SpunkPipe", &format!("{e:#}"));
        eprintln!("spunkpipe-tray: {e:#}");
        std::process::exit(1);
    }
}

fn start() -> anyhow::Result<()> {
    // A clicked invite link starts us with the link as the only argument.
    if let Some(url) = std::env::args().nth(1).filter(|a| link::is_link(a)) {
        link::join_from_link(&url)?;
    }
    // One tray per user. After a restart the old one may still be exiting, so wait a little.
    let Some(_lock) = single::acquire(std::time::Duration::from_secs(10)) else {
        return Ok(()); // already running
    };
    run()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(any(windows, target_os = "macos"))]
fn run() -> anyhow::Result<()> {
    desktop::run()
}
