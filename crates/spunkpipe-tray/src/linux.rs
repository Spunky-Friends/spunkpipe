//! Linux and other Unix desktops: a StatusNotifierItem tray over D-Bus (no GTK needed).
//! KDE, XFCE and others show it natively; GNOME needs the AppIndicator extension. If there is
//! no tray at all, the updater keeps running with desktop notifications only.

use crate::app::{App, Exit, Refresh};
use crate::icon;
use crate::menu::{build_menu, needs_attention, tooltip, Item};
use anyhow::Result;
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip, Tray};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct SpunkTray {
    app: Arc<App>,
}

fn pixmaps() -> Vec<Icon> {
    [22, 32, 48, 64].iter().map(|&s| Icon { width: s as i32, height: s as i32, data: icon::argb(s) }).collect()
}

impl Tray for SpunkTray {
    fn id(&self) -> String {
        "spunkpipe".into()
    }
    fn title(&self) -> String {
        "SpunkPipe".into()
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        pixmaps()
    }
    fn status(&self) -> ksni::Status {
        if needs_attention(&self.app.views()) {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }
    fn attention_icon_pixmap(&self) -> Vec<Icon> {
        pixmaps()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "SpunkPipe".into(), description: tooltip(&self.app.views()), ..Default::default() }
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        build_menu(&self.app.views())
            .into_iter()
            .map(|item| match item {
                Item::Separator => MenuItem::Separator,
                Item::Label(label) => StandardItem { label, enabled: false, ..Default::default() }.into(),
                Item::Action { label, enabled, action } => StandardItem {
                    label,
                    enabled,
                    activate: Box::new(move |t: &mut SpunkTray| t.app.handle(action.clone())),
                    ..Default::default()
                }
                .into(),
                Item::Choice { label, checked, action } => CheckmarkItem {
                    label,
                    checked,
                    activate: Box::new(move |t: &mut SpunkTray| t.app.handle(action.clone())),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

pub fn run() -> Result<()> {
    let refresh: Refresh = Arc::new(Mutex::new(Box::new(|| {})));
    let (app, quit) = App::start(refresh.clone())?;

    match (SpunkTray { app: app.clone() }).spawn() {
        Ok(handle) => {
            let h = handle.clone();
            *refresh.lock().unwrap() = Box::new(move || {
                h.update(|_| {});
            });
            // Keep the percentage in the tooltip moving while a download runs.
            let (h, a) = (handle.clone(), app.clone());
            std::thread::spawn(move || {
                while !h.is_closed() {
                    std::thread::sleep(Duration::from_secs(2));
                    if a.downloading() {
                        h.update(|_| {});
                    }
                }
            });
            let why = quit.recv().unwrap_or(Exit::Quit);
            handle.shutdown().wait();
            app.finish(why);
        }
        Err(e) => {
            eprintln!("spunkpipe-tray: no system tray available ({e}); running with notifications only");
            let why = quit.recv().unwrap_or(Exit::Quit);
            app.finish(why);
        }
    }
    Ok(())
}
