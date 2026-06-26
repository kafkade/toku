//! Tauri v2 desktop shell for the Toku web UI.
//!
//! Architecture:
//!   1. Bind a TCP listener on a random localhost port.
//!   2. Spawn the Axum web server on a background thread.
//!   3. Open a Tauri WebView window pointing at the local server.
//!   4. Add a system-tray icon with Show / Resolve conflicts / Quit items.
//!   5. Poll the local database for unresolved sync conflicts and raise a
//!      system notification (and update the tray tooltip) when new conflicts
//!      appear.
//!
//! Closing the window hides it to the system tray. The "Quit" menu
//! item (or closing via Alt-F4 while the tray is focused) exits the
//! process and tears down the background server thread.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// Tray icon identifier, used to look the tray up for tooltip updates.
const TRAY_ID: &str = "toku-tray";

/// How often to poll the local database for unresolved sync conflicts.
const CONFLICT_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Launch the Toku desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        // Hide (don't destroy) the window when the user clicks the X button
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // ── 1. Bind a random available port ─────────────────────
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            listener.set_nonblocking(true)?;

            // ── 2. Resolve data directory ───────────────────────────
            let data_dir = toku_db::Database::default_data_dir()?;
            let db_path = data_dir.join("toku.db");
            let watch_db_path = db_path.clone();

            // ── 3. Start the Axum web server ────────────────────────
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build tokio runtime");
                rt.block_on(async move {
                    let tokio_listener = tokio::net::TcpListener::from_std(listener)
                        .expect("failed to convert TCP listener");
                    if let Err(e) = toku_web::serve_on(db_path, tokio_listener).await {
                        eprintln!("toku web server error: {e}");
                    }
                });
            });

            // ── 4. Create the main application window ───────────────
            let url = format!("http://127.0.0.1:{port}");
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("valid localhost URL")),
            )
            .title("Toku")
            .inner_size(1200.0, 800.0)
            .min_inner_size(800.0, 600.0)
            .center()
            .build()?;

            // ── 5. System tray ──────────────────────────────────────
            let show_item = MenuItem::with_id(app, "show", "Show Toku", true, None::<&str>)?;
            let conflicts_item =
                MenuItem::with_id(app, "conflicts", "Resolve conflicts…", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &conflicts_item, &quit_item])?;

            // 16×16 blue placeholder icon (RGBA) — replace with a
            // proper app icon before shipping the installer.
            let icon_rgba = [0x4Au8, 0x82, 0xCF, 0xFF].repeat(16 * 16);
            let tray_icon = Image::new_owned(icon_rgba, 16, 16);

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(tray_icon)
                .menu(&tray_menu)
                .tooltip("Toku — Book Manager")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "conflicts" => show_conflicts(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // ── 6. Watch for sync conflicts ─────────────────────────
            spawn_conflict_watcher(app.handle().clone(), watch_db_path);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Toku desktop");
}

/// Show and focus the main window.
fn show_main_window(app: &impl Manager<tauri::Wry>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Show the main window and navigate the web view to the conflicts page.
fn show_conflicts(app: &AppHandle) {
    show_main_window(app);
    if let Some(w) = app.get_webview_window("main") {
        // Relative navigation resolves against the local server origin.
        let _ = w.eval("window.location.assign('/conflicts')");
    }
}

/// Periodically poll the local database for unresolved sync conflicts.
///
/// On a rising edge (a new conflict appearing since the last notification) a
/// system notification is raised. The tray tooltip is kept in sync with the
/// current count on every poll. All errors are swallowed — the watcher must
/// never crash the app.
fn spawn_conflict_watcher(app: AppHandle, db_path: PathBuf) {
    std::thread::spawn(move || {
        let mut last_notified: i64 = 0;

        loop {
            std::thread::sleep(CONFLICT_POLL_INTERVAL);

            let count = match unresolved_conflict_count(&db_path) {
                Some(c) => c,
                None => continue,
            };

            // Keep the tray tooltip current.
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                let tooltip = if count > 0 {
                    format!(
                        "Toku — {count} sync conflict{} need review",
                        if count == 1 { "" } else { "s" }
                    )
                } else {
                    "Toku — Book Manager".to_string()
                };
                let _ = tray.set_tooltip(Some(&tooltip));
            }

            // Notify only when new conflicts have appeared since last time.
            if count > last_notified {
                let _ = app
                    .notification()
                    .builder()
                    .title("Toku — sync conflicts")
                    .body(format!(
                        "{count} sync conflict{} need your review.",
                        if count == 1 { "" } else { "s" }
                    ))
                    .show();
            }
            last_notified = count;
        }
    });
}

/// Count unresolved sync conflicts in the local database, or `None` on error.
fn unresolved_conflict_count(db_path: &Path) -> Option<i64> {
    let db = toku_db::Database::open_no_migrate(db_path).ok()?;
    toku_db::SyncRepository::new(&db)
        .count_unresolved_conflicts()
        .ok()
}
