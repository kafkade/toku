//! Tauri v2 desktop shell for the Toku web UI.
//!
//! Architecture:
//!   1. Bind a TCP listener on a random localhost port.
//!   2. Spawn the Axum web server on a background thread.
//!   3. Open a Tauri WebView window pointing at the local server.
//!   4. Add a system-tray icon with Show / Quit menu items.
//!
//! Closing the window hides it to the system tray. The "Quit" menu
//! item (or closing via Alt-F4 while the tray is focused) exits the
//! process and tears down the background server thread.

use std::net::TcpListener;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Launch the Toku desktop application.
pub fn run() {
    tauri::Builder::default()
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
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // 16×16 blue placeholder icon (RGBA) — replace with a
            // proper app icon before shipping the installer.
            let icon_rgba = [0x4Au8, 0x82, 0xCF, 0xFF].repeat(16 * 16);
            let tray_icon = Image::new_owned(icon_rgba, 16, 16);

            TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&tray_menu)
                .tooltip("Toku — Book Manager")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

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
