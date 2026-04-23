//! `c2pa-tui` — terminal UI for browsing and comparing C2PA manifests.

/// Top-level application state and event loop.
pub mod app;
/// Manifest comparison / diff engine.
pub mod compare;
/// Runtime configuration.
pub mod config;
/// Error types shared across the crate.
pub mod error;
/// Manifest loading, tree representation, and filtering.
pub mod manifest;
/// Remote HTTP client and authentication.
pub mod remote;
/// Fuzzy search over manifest fields.
pub mod search;
#[allow(dead_code)]
pub(crate) mod ui;
