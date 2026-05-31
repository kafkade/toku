//! Toku Desktop — Windows desktop application.
//!
//! Wraps the Phase 4 web UI (Axum + maud) in a native Tauri v2 window.
//! On non-Windows platforms this binary prints a platform-not-supported
//! message and exits.

#[cfg(windows)]
mod desktop;

#[cfg(not(windows))]
fn main() {
    eprintln!("toku-desktop is only supported on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    desktop::run();
}
