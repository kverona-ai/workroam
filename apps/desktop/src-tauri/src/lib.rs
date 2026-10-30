//! Workroam desktop app (ADR-0002): windows, tray and a bridge to
//! `workroamd`. It holds no sync logic; every action is an IPC call the
//! page makes through the `ipc` command, the same calls the CLI makes.

mod bridge;
mod tray;

use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

/// One IPC call to the daemon. Errors are `workroam_core::Error` as JSON
/// (`code`, `message`, `hint`, `data`), the same shape the CLI prints.
#[tauri::command]
async fn ipc(app: AppHandle, method: String, params: Option<Value>) -> Result<Value, Value> {
    bridge::call(&app, &method, params.unwrap_or(Value::Null))
        .await
        .map_err(|e| serde_json::to_value(&e).unwrap_or_default())
}

/// Whether the daemon connection is up (the page shows a banner otherwise).
#[tauri::command]
async fn daemon_state(app: AppHandle) -> bool {
    bridge::connected(&app).await
}

#[derive(Deserialize)]
struct TrayModel {
    /// idle, syncing or attention.
    state: String,
    tooltip: String,
    items: Vec<tray::Entry>,
}

/// The page computes the tray menu from the daemon state; Rust only draws it.
#[tauri::command]
fn tray_update(app: AppHandle, model: TrayModel) -> Result<(), String> {
    tray::update(&app, &model.state, &model.tooltip, &model.items).map_err(|e| e.to_string())
}

/// Size and place the corner-card window: top right on macOS (below the
/// menu bar), bottom right elsewhere (above the taskbar). Height 0 hides it.
#[tauri::command]
fn cards_layout(app: AppHandle, height: f64) -> Result<(), String> {
    let Some(w) = app.get_webview_window("cards") else {
        return Ok(());
    };
    if height <= 0.0 {
        return w.hide().map_err(|e| e.to_string());
    }
    let mon = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten());
    if let Some(m) = mon {
        let scale = m.scale_factor();
        let area = m.work_area();
        let width = (372.0 * scale) as u32;
        let h = ((height + 16.0) * scale).min(area.size.height as f64) as u32;
        let margin = (8.0 * scale) as i32;
        let x = area.position.x + area.size.width as i32 - width as i32 - margin;
        let y = if cfg!(target_os = "macos") {
            area.position.y + margin
        } else {
            area.position.y + area.size.height as i32 - h as i32 - margin
        };
        let _ = w.set_size(PhysicalSize::new(width, h));
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
    // Shown without taking focus (UX-05): the window is created unfocusable.
    w.show().map_err(|e| e.to_string())
}

/// Open (or focus) a secondary window, e.g. the approval details window.
#[tauri::command]
fn open_window(
    app: AppHandle,
    label: String,
    route: String,
    title: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if !label.starts_with("details-") {
        return Err("only details windows can be opened".into());
    }
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::App(format!("index.html#{route}").into()),
    )
    .title(title)
    .inner_size(width, height)
    .min_inner_size(420.0, 360.0)
    .center()
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Show the main window, optionally on a page (`projects`, `doctor`…).
#[tauri::command]
fn show_main(app: AppHandle, page: Option<String>) {
    tray::show_main(&app, page.as_deref());
}

/// Write a text file the person chose in a save dialog (recovery key).
/// The file is private to the user where the system supports it.
#[tauri::command]
fn save_private_text(path: String, text: String) -> Result<(), String> {
    write_private(std::path::Path::new(&path), text.as_bytes()).map_err(|e| e.to_string())
}

fn write_private(path: &std::path::Path, body: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(body)
    }
    #[cfg(not(unix))]
    std::fs::write(path, body)
}

/// Decode a QR code from a grayscale camera frame or image (pairing code).
#[tauri::command]
fn decode_qr(width: usize, height: usize, luma: Vec<u8>) -> Option<String> {
    if width == 0 || height == 0 || luma.len() < width * height {
        return None;
    }
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| luma[y * width + x]);
    img.detect_grids()
        .into_iter()
        .find_map(|g| g.decode().ok().map(|(_, text)| text))
}

pub fn run() {
    let hidden = std::env::args().any(|a| a == "--autostart");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app, None);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(bridge::Bridge::default())
        .invoke_handler(tauri::generate_handler![
            ipc,
            daemon_state,
            tray_update,
            cards_layout,
            open_window,
            show_main,
            save_private_text,
            decode_qr,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::create(&handle)?;
            bridge::spawn(handle.clone());
            if !hidden {
                tray::show_main(&handle, None);
            }
            Ok(())
        })
        .on_window_event(|w, ev| {
            // Closing the main window keeps Workroam in the tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                if w.label() == "main" {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the Workroam app")
        .run(|_app, _ev| {});
}
