//! `client add|revoke|require-token` and `clients`: agents with their own token.

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use hostlane_core::client::Client;
use hostlane_protocol::{ClientCreated, ClientsList, method};
use serde_json::{Value, json};

use crate::{ago, print_json, print_table};

#[derive(Subcommand)]
pub enum ClientsCommand {
    /// Register agents with their own token, so the audit log shows who called.
    #[command(subcommand)]
    Client(ClientCommand),
    /// List registered agents (never their tokens).
    Clients,
}

#[derive(Subcommand)]
pub enum ClientCommand {
    /// Register an agent and print its token (shown only once).
    Add {
        /// Name shown in the audit log, e.g. `claude-code`.
        name: String,
    },
    /// Revoke an agent's token.
    Revoke { name: String },
    /// Require a token from every agent (any client except `cli` and `desktop`).
    RequireToken {
        #[arg(value_enum)]
        state: Switch,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Switch {
    On,
    Off,
}

fn print_list(list: &ClientsList) {
    if list.clients.is_empty() {
        println!("No registered clients.");
    } else {
        let rows: Vec<[String; 3]> = list
            .clients
            .iter()
            .map(|c| {
                [
                    c.name.clone(),
                    ago(c.created_at),
                    c.last_used_at.map(ago).unwrap_or_else(|| "never".into()),
                ]
            })
            .collect();
        print_table(["CLIENT", "CREATED", "LAST USED"], &rows);
    }
    println!(
        "Tokens required for agents: {}",
        if list.require_token { "yes" } else { "no" }
    );
}

pub async fn run(client: &Client, command: ClientsCommand, json: bool) -> Result<()> {
    match command {
        ClientsCommand::Clients => {
            let list: ClientsList = client.call_as(method::CLIENTS_LIST, Value::Null).await?;
            if json {
                print_json(&list);
            } else {
                print_list(&list);
            }
        }
        ClientsCommand::Client(ClientCommand::Add { name }) => {
            let created: ClientCreated = client
                .call_as(method::CLIENTS_CREATE, json!({ "name": name }))
                .await?;
            if json {
                print_json(&created);
            } else {
                println!(
                    "Registered `{}`. Its token is shown only this once:\n\n  {}\n",
                    created.client.name, created.token
                );
                println!(
                    "Give it to the agent as HOSTLANE_TOKEN (in the env of `hostlane mcp`, or of the shell that runs hostlane)."
                );
            }
        }
        ClientsCommand::Client(ClientCommand::Revoke { name }) => {
            let reply = client
                .call(method::CLIENTS_REVOKE, json!({ "name": name }))
                .await?;
            crate::output_value(json, reply, &format!("Revoked the token of `{name}`."));
        }
        ClientsCommand::Client(ClientCommand::RequireToken { state }) => {
            let require = matches!(state, Switch::On);
            let list: ClientsList = client
                .call_as(
                    method::CLIENTS_CONFIGURE,
                    json!({ "require_token": require }),
                )
                .await?;
            if json {
                print_json(&list);
            } else {
                print_list(&list);
            }
        }
    }
    Ok(())
}
