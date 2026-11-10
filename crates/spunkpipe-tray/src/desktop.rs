//! Windows and macOS: a native tray icon driven by a small event loop.

use crate::app::{App, Refresh};
use crate::icon;
use crate::menu::{build_menu, tooltip, Action, Item};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

enum UserEvent {
    Refresh,
    Menu(MenuEvent),
}

fn make_icon() -> Result<Icon> {
    Icon::from_rgba(icon::rgba(64), 64, 64).context("building the tray icon")
}

/// Builds the menu and remembers which menu id means which action.
fn build(app: &App) -> Result<(Menu, HashMap<MenuId, Action>, String)> {
    let views = app.views();
    let menu = Menu::new();
    let mut actions = HashMap::new();
    for item in build_menu(&views) {
        match item {
            Item::Separator => menu.append(&PredefinedMenuItem::separator())?,
            Item::Label(t) => menu.append(&MenuItem::new(t, false, None))?,
            Item::Action { label, enabled, action } => {
                let m = MenuItem::new(label, enabled, None);
                actions.insert(m.id().clone(), action);
                menu.append(&m)?;
            }
            Item::Choice { label, checked, action } => {
                let m = CheckMenuItem::new(label, true, checked, None);
                actions.insert(m.id().clone(), action);
                menu.append(&m)?;
            }
        }
    }
    Ok((menu, actions, tooltip(&views)))
}

pub fn run() -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    let event_loop = {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        // Menu bar only, no Dock icon.
        let mut event_loop = event_loop;
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop
    };
    run_loop(event_loop)
}

fn run_loop(event_loop: tao::event_loop::EventLoop<UserEvent>) -> Result<()> {
    let proxy = event_loop.create_proxy();
    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::Menu(e));
    }));

    let refresh: Refresh = {
        let p = proxy.clone();
        Arc::new(Mutex::new(Box::new(move || {
            let _ = p.send_event(UserEvent::Refresh);
        })))
    };
    let (app, quit) = App::start(refresh)?;

    // Keep the percentage moving during a download, and notice Quit from any thread.
    {
        let (p, a) = (proxy.clone(), app.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            if a.downloading() {
                let _ = p.send_event(UserEvent::Refresh);
            }
        });
    }

    let mut tray: Option<TrayIcon> = None;
    let mut actions: HashMap<MenuId, Action> = HashMap::new();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if quit.try_recv().is_ok() {
            app.shutdown();
            *control_flow = ControlFlow::Exit;
            return;
        }
        match event {
            // The tray icon must be created once the loop is running (required on macOS).
            Event::NewEvents(StartCause::Init) => match (build(&app), make_icon()) {
                (Ok((menu, acts, tip)), Ok(icon)) => {
                    actions = acts;
                    tray = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip(tip)
                        .with_icon(icon)
                        .build()
                        .map_err(|e| eprintln!("spunkpipe-tray: could not create the tray icon: {e}"))
                        .ok();
                }
                (Err(e), _) | (_, Err(e)) => eprintln!("spunkpipe-tray: {e:#}"),
            },
            Event::UserEvent(UserEvent::Refresh) => {
                if let (Some(t), Ok((menu, acts, tip))) = (&tray, build(&app)) {
                    actions = acts;
                    t.set_menu(Some(Box::new(menu)));
                    let _ = t.set_tooltip(Some(tip));
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if let Some(a) = actions.get(&e.id) {
                    let quitting = *a == Action::Quit;
                    app.handle(a.clone());
                    if quitting {
                        app.shutdown();
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            _ => {}
        }
    });
}
