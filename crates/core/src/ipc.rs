//! Local channel between the app and the browser helper: one JSON line each way
//! over a loopback socket, authorised by a token only this user can read.

use crate::manager::Manager;
use crate::types::DownloadRequest;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const MAX_MESSAGE: u64 = 4 * 1024 * 1024;
const ENDPOINT_FILE: &str = "ipc.json";

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: String,
}

/// A download described by the browser. It deliberately cannot choose a folder.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrowserItem {
    pub url: String,
    pub referrer: Option<String>,
    pub cookie: Option<String>,
    pub user_agent: Option<String>,
    pub filename: Option<String>,
}

impl From<BrowserItem> for DownloadRequest {
    fn from(i: BrowserItem) -> Self {
        let clean = |v: Option<String>| v.filter(|s| !s.is_empty() && !s.contains(['\r', '\n', '\0']));
        DownloadRequest {
            url: i.url,
            referrer: clean(i.referrer),
            cookie: clean(i.cookie),
            user_agent: clean(i.user_agent),
            filename: i.filename.as_deref().and_then(crate::naming::sanitize),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Command {
    Ping,
    Add {
        items: Vec<BrowserItem>,
        /// Already chosen by the user in the link picker: do not ask again.
        #[serde(default)]
        confirmed: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    token: String,
    #[serde(flatten)]
    command: Command,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default)]
    pub accepted: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn new_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn handle(manager: &Manager, command: Command) -> Reply {
    match command {
        Command::Ping => Reply { ok: true, ..Default::default() },
        Command::Add { items, confirmed } => {
            let mut accepted = 0;
            let mut error = None;
            for item in items {
                let result = if confirmed {
                    manager.add(item.into(), true).map(drop)
                } else {
                    manager.offer(item.into())
                };
                match result {
                    Ok(()) => accepted += 1,
                    Err(e) => error = Some(e.to_string()),
                }
            }
            Reply { ok: accepted > 0, accepted, error }
        }
    }
}

/// Bind the socket and publish its address. `on_message` lets the UI come
/// forward when the browser sends something.
pub async fn serve(manager: Arc<Manager>, on_message: Arc<dyn Fn() + Send + Sync>) -> Result<()> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let endpoint = Endpoint { port: listener.local_addr()?.port(), token: new_token()? };
    crate::fsio::write_private(
        &manager.data_dir().join(ENDPOINT_FILE),
        &serde_json::to_vec(&endpoint)?,
    )?;
    let token = Arc::new(endpoint.token);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            };
            let (manager, token, notify) = (manager.clone(), token.clone(), on_message.clone());
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                let mut reader = tokio::io::BufReader::new(read.take(MAX_MESSAGE));
                let got = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line));
                if !matches!(got.await, Ok(Ok(n)) if n > 0) {
                    return;
                }
                let reply = match serde_json::from_str::<Envelope>(&line) {
                    Ok(env) if env.token == *token => {
                        let adds = matches!(env.command, Command::Add { .. });
                        let reply = handle(&manager, env.command);
                        if adds {
                            notify();
                        }
                        reply
                    }
                    Ok(_) => Reply { error: Some("unauthorised".into()), ..Default::default() },
                    Err(_) => Reply { error: Some("bad request".into()), ..Default::default() },
                };
                if let Ok(mut out) = serde_json::to_vec(&reply) {
                    out.push(b'\n');
                    let _ = write.write_all(&out).await;
                }
            });
        }
    });
    Ok(())
}

/// Blocking client used by the browser helper and the single-instance check.
pub fn request(data_dir: &Path, command: Command) -> Result<Reply> {
    let data = std::fs::read(data_dir.join(ENDPOINT_FILE)).context("FluxDM is not running")?;
    let endpoint: Endpoint = serde_json::from_slice(&data)?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .context("FluxDM is not running")?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut line = serde_json::to_vec(&Envelope { token: endpoint.token, command })?;
    if line.len() as u64 >= MAX_MESSAGE {
        bail!("message too large");
    }
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reply = String::new();
    BufReader::new(stream.take(MAX_MESSAGE)).read_line(&mut reply)?;
    Ok(serde_json::from_str(&reply)?)
}
