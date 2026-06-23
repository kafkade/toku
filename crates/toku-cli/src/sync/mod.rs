//! Sync support for the CLI.
//!
//! The reusable sync logic (HTTP client, credential storage, wire format, and the
//! high-level push/pull/init/status orchestration) lives in the `toku-sync-client`
//! crate so it can be shared with the FFI layer and other frontends. The CLI re-exports
//! those modules here and adds only command-line presentation on top.

pub use toku_sync_client::orchestrator;
pub use toku_sync_client::{client, token_store};
