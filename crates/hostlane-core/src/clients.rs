//! A5: agents registered with a name and a token. A call carrying a valid token is
//! recorded under the registered name and marked verified, whatever name it claims.
//! With `require_token` on, agents (any name except `cli` and `desktop`) must send one.
//! Hostlane keeps only the SHA-256 of each token.

use std::sync::atomic::{AtomicBool, Ordering};

use hostlane_protocol::{
    ClientCreated, ClientInfo, ClientsList, ErrorKind, RegisteredClient, RpcError, method,
};

use crate::store::Store;
use crate::util::{now_ms, sha256_hex};

const REQUIRE_SETTING: &str = "clients.require_token";
const TOKEN_PREFIX: &str = "hlt_";
/// Names that mean a person; a registered agent never gets them.
const RESERVED: &[&str] = &["cli", "desktop", "unknown"];

pub struct Clients {
    store: Store,
    require_token: AtomicBool,
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn check_name(name: &str) -> Result<String, RpcError> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        return Err(invalid(
            "a client name is 1–64 letters, digits, `.`, `_` or `-`",
        ));
    }
    if RESERVED.contains(&name) {
        return Err(invalid(format!("`{name}` is reserved for people")));
    }
    Ok(name.to_string())
}

fn new_token() -> Result<String, RpcError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|err| RpcError::new(ErrorKind::Internal, format!("no randomness: {err}")))?;
    Ok(format!(
        "{TOKEN_PREFIX}{}",
        data_encoding::HEXLOWER.encode(&bytes)
    ))
}

impl Clients {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            require_token: AtomicBool::new(false),
        }
    }

    pub async fn load(&self) {
        match self.store.setting(REQUIRE_SETTING).await {
            Ok(value) => self
                .require_token
                .store(value.as_deref() == Some("true"), Ordering::SeqCst),
            Err(err) => tracing::error!(%err, "cannot load the client token setting"),
        }
    }

    pub fn require_token(&self) -> bool {
        self.require_token.load(Ordering::SeqCst)
    }

    /// Who is calling: the registered name for a valid token, otherwise the claimed
    /// name, unverified.
    pub async fn authenticate(
        &self,
        mut client: ClientInfo,
        token: Option<&str>,
        name: &str,
    ) -> Result<ClientInfo, RpcError> {
        client.verified = false;
        if let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) {
            let hash = sha256_hex(token.as_bytes());
            return match self.store.use_client_token(hash, now_ms()).await? {
                Some(registered) => {
                    client.name = registered;
                    client.verified = true;
                    Ok(client)
                }
                None => Err(RpcError::new(
                    ErrorKind::Unauthorized,
                    "the client token is unknown or was revoked",
                )
                .with_hint(
                    "ask the user for a new token (Hostlane app → Integrations, or `hostlane client add <name>`) and set it as HOSTLANE_TOKEN",
                )),
            };
        }
        let person = RESERVED[..2].contains(&client.name.as_str());
        if self.require_token() && !person && name != method::DAEMON_STATUS {
            return Err(RpcError::new(
                ErrorKind::Unauthorized,
                format!("`{}` sent no client token, and Hostlane requires one", client.name),
            )
            .with_hint(
                "ask the user to register this agent (Hostlane app → Integrations, or `hostlane client add <name>`) and set HOSTLANE_TOKEN",
            ));
        }
        Ok(client)
    }

    pub async fn list(&self) -> Result<ClientsList, RpcError> {
        Ok(ClientsList {
            clients: self.store.clients().await?,
            require_token: self.require_token(),
        })
    }

    pub async fn create(&self, name: &str) -> Result<ClientCreated, RpcError> {
        let name = check_name(name)?;
        let token = new_token()?;
        let client = RegisteredClient {
            name: name.clone(),
            created_at: now_ms(),
            last_used_at: None,
        };
        let added = self
            .store
            .insert_client(client.clone(), sha256_hex(token.as_bytes()))
            .await?;
        if !added {
            return Err(invalid(format!("a client named `{name}` already exists"))
                .with_hint("revoke it first to issue a new token, or pick another name"));
        }
        Ok(ClientCreated { client, token })
    }

    pub async fn revoke(&self, name: &str) -> Result<(), RpcError> {
        if self.store.delete_client(name.trim().to_string()).await? {
            Ok(())
        } else {
            Err(invalid(format!("no client named `{name}`"))
                .with_hint("list them with clients.list (CLI: hostlane clients)"))
        }
    }

    pub async fn configure(&self, require_token: bool) -> Result<ClientsList, RpcError> {
        self.store
            .set_setting(
                REQUIRE_SETTING.into(),
                require_token.then(|| "true".to_string()),
            )
            .await?;
        self.require_token.store(require_token, Ordering::SeqCst);
        self.list().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clients() -> Clients {
        Clients::new(Store::open_in_memory().unwrap())
    }

    #[tokio::test]
    async fn tokens_identify_registered_clients() {
        let clients = clients();
        let created = clients.create("claude-code").await.unwrap();
        assert!(created.token.starts_with(TOKEN_PREFIX));
        assert!(clients.create("claude-code").await.is_err());
        assert!(clients.create("desktop").await.is_err());
        assert!(clients.create("a b").await.is_err());

        let claimed = ClientInfo {
            name: "desktop".into(),
            session: Some("s1".into()),
            verified: true,
        };
        let who = clients
            .authenticate(claimed.clone(), Some(&created.token), method::EXEC_RUN)
            .await
            .unwrap();
        assert_eq!(who.name, "claude-code");
        assert!(who.verified);
        assert_eq!(who.session.as_deref(), Some("s1"));

        let unverified = clients
            .authenticate(claimed, None, method::EXEC_RUN)
            .await
            .unwrap();
        assert!(!unverified.verified);

        let listed = clients.list().await.unwrap();
        assert!(listed.clients[0].last_used_at.is_some());

        clients.revoke("claude-code").await.unwrap();
        let err = clients
            .authenticate(ClientInfo::new("x"), Some(&created.token), method::EXEC_RUN)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
    }

    #[tokio::test]
    async fn require_token_refuses_anonymous_agents() {
        let clients = clients();
        clients.configure(true).await.unwrap();
        let err = clients
            .authenticate(ClientInfo::new("codex"), None, method::EXEC_RUN)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
        for name in ["cli", "desktop"] {
            assert!(
                clients
                    .authenticate(ClientInfo::new(name), None, method::EXEC_RUN)
                    .await
                    .is_ok()
            );
        }
        assert!(
            clients
                .authenticate(ClientInfo::new("codex"), None, method::DAEMON_STATUS)
                .await
                .is_ok()
        );
        clients.configure(false).await.unwrap();
        clients.load().await;
        assert!(!clients.require_token());
    }
}
