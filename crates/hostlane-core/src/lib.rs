//! Core of the Hostlane daemon. The `hostlane` binary wires this up behind
//! `hostlane daemon run`; clients talk to it through [`client::Client`].

pub mod client;
pub mod known_hosts;
pub mod paths;
pub mod server;
pub mod ssh_config;

mod events;
mod exec;
mod files;
mod handlers;
mod patch;
mod policy;
mod pool;
mod ports;
mod remote;
mod ssh;
mod state;
mod store;
mod tunnels;
mod util;

pub use state::{Daemon, DaemonOptions, OpenError};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
