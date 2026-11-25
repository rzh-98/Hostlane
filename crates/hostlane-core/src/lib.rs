//! Core of the Hostlane daemon. The `hostlane` binary wires this up behind
//! `hostlane daemon run`; clients talk to it through [`client::Client`].

pub mod client;
pub mod known_hosts;
pub mod paths;
pub mod secrets;
pub mod server;
pub mod ssh_config;

mod catalog;
mod clients;
mod events;
mod exec;
mod files;
mod handlers;
mod hosts;
mod jobs;
mod patch;
mod policy;
mod pool;
mod ports;
mod remote;
mod services;
mod ssh;
mod state;
mod store;
mod tunnels;
mod util;

pub use state::{Daemon, DaemonOptions, OpenError};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
