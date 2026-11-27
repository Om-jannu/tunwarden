//! Core library for Tunwarden: config, tunnel types, supervisor, daemon and
//! protocol. Contains no UI code. See `docs/technical-design.md`.

pub mod auth;
pub mod autostart;
pub mod client;
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod failure;
pub mod fsutil;
pub mod import;
pub mod ipc;
pub mod logs;
pub mod paths;
pub mod persist;
pub mod probe;
pub mod process;
pub mod proto;
pub mod state;
pub mod supervisor;
pub mod system;
pub mod tools;
pub mod types;

/// Version of this crate, reported in the daemon handshake.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
