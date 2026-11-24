//! Core of the Hostlane daemon. The `hostlane` binary wires this up behind
//! `hostlane daemon run`; clients talk to it through [`client::Client`].

pub mod client;
pub mod paths;
pub mod server;
pub mod ssh_config;

mod handlers;
mod state;

pub use state::Daemon;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
