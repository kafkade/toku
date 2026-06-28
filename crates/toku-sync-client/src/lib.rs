//! Reusable sync client for Toku.
//!
//! This crate contains the platform-agnostic pieces of the sync feature that are shared
//! between the CLI binary and the FFI layer (and, in the future, other frontends):
//!
//! - [`client`] — HTTP client for the `toku-sync` server.
//! - [`token_store`] — OS keychain (with file fallback) storage for auth tokens and keys.
//! - [`wire`] — conversion between domain `SyncOp`s and the wire format.
//! - [`orchestrator`] — high-level `init`/`push`/`pull`/`status`/`devices` flows that
//!   return structured outcomes without performing any I/O presentation.

pub mod client;
pub mod orchestrator;
pub mod token_store;
pub mod wire;

pub use client::{
    AccountDeviceInfo, AccountKeyBundle, DeviceInfo, EnrollResponse, SrpChallengeResponse,
    SrpVerifyResponse, SyncClient, WireOp,
};
pub use orchestrator::{
    BootstrapOutcome, EnrollOutcome, InitOutcome, LoginOutcome, PullOutcome, PushOutcome,
    SignupOutcome, StatusOutcome, account_devices, bootstrap, conflict, conflicts,
    default_device_name, devices, enroll, init, login, pull, push, resolve_all_conflicts,
    resolve_conflict, signup, status,
};
pub use token_store::TokenStore;
