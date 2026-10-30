//! The tray icon and its menu. The main window's page builds the menu model
//! from the daemon state and handles the clicks (`tray` event), so the tray
//! and the windows run the same actions.

use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

const TRAY_ID: &str = "workroam";

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Entry {
    /// A disabled line of text (the header, a project's state).
    Text {
        text: String,
    },
    Item {
        id: String,
        text: String,
        #[serde(default = "yes")]
        enabled: bool,
    },
    Sep,
    Submenu {
        text: String,
        items: Vec<Entry>,
    },
}

fn yes() -> bool {
    true
}

fn icon(state: &str) -> Option<Image<'static>> {
    let bytes: &'static [u8] = match state {
        "syncing" => include_bytes!("../icons/tray-sync.png"),
        "attention" => include_bytes!("../icons/tray-alert.png"),
        _ => include_bytes!("../icons/tray.png"),
    };
    Image::from_bytes(bytes).ok()
}

fn build<R: Runtime>(
    app: &AppHandle<R>,
    items: &[Entry],
) -> tauri::Result<Vec<Box<dyn IsMenuItem<R>>>> {
    let mut out: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    for it in items {
        match it {
            Entry::Text { text } => {
                out.push(Box::new(MenuItem::new(app, text, false, None::<&str>)?))
            }
            Entry::Item { id, text, enabled } => out.push(Box::new(MenuItem::with_id(
                app,
                id,
                text,
                *enabled,
                None::<&str>,
            )?)),
            Entry::Sep => out.push(Box::new(PredefinedMenuItem::separator(app)?)),
            Entry::Submenu { text, items } => {
                let kids = build(app, items)?;
                let refs: Vec<&dyn IsMenuItem<R>> = kids.iter().map(|b| b.as_ref()).collect();
                out.push(Box::new(Submenu::with_items(app, text, true, &refs)?));
            }
        }
    }
    Ok(out)
}

fn menu(app: &AppHandle, items: &[Entry]) -> tauri::Result<Menu<Wry>> {
    let mut all = build(app, items)?;
    if !items.is_empty() {
        all.push(Box::new(PredefinedMenuItem::separator(app)?));
    }
    all.push(Box::new(MenuItem::with_id(
        app,
        "open",
        "Workroam…",
        true,
        None::<&str>,
    )?));
    all.push(Box::new(MenuItem::with_id(
        app,
        "quit",
        "Quit / 退出",
        true,
        None::<&str>,
    )?));
    let refs: Vec<&dyn IsMenuItem<Wry>> = all.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let mut b = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Workroam")
        .menu(&menu(app, &[])?)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "quit" => app.exit(0),
            "open" => show_main(app, None),
            id => {
                // Actions run in the main window's page.
                let _ = app.emit_to("main", "tray", id.to_string());
            }
        });
    if let Some(i) = icon("idle") {
        b = b.icon(i).icon_as_template(true);
    }
    b.build(app)?;
    Ok(())
}

pub fn update(app: &AppHandle, state: &str, tooltip: &str, items: &[Entry]) -> tauri::Result<()> {
    let Some(t) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    t.set_menu(Some(menu(app, items)?))?;
    t.set_tooltip(Some(tooltip))?;
    if let Some(i) = icon(state) {
        t.set_icon(Some(i))?;
        // The alert badge is coloured; the others follow the menu bar.
        t.set_icon_as_template(state != "attention")?;
    }
    Ok(())
}

pub fn show_main(app: &AppHandle, page: Option<&str>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        if let Some(p) = page {
            let _ = app.emit_to("main", "navigate", p.to_string());
        }
    }
}
