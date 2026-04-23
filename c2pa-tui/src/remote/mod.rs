/// HTTP authentication variants.
pub mod auth;
/// Shared HTTP client for remote manifest fetching.
pub mod client;

pub use auth::Auth;
pub use client::RemoteClient;
