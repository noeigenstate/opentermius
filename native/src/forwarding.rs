//! Independent, bounded SSH tunnels and a minimal RFC 1928 CONNECT proxy.
use crate::{ssh, store::Profile};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    runtime::Handle,
    sync::{mpsc, watch},
    task::{JoinHandle, JoinSet},
    time::{interval, timeout},
};
use zeroize::Zeroizing;

pub const MAX_CONNECTIONS: usize = 256;
pub const MAX_RULES: usize = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Local,
    Remote,
    Dynamic,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Forward {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub host_id: String,
    pub bind_address: String,
    pub bind_port: u16,
    pub destination_host: String,
    pub destination_port: u16,
    pub auto_start: bool,
}
impl Default for Forward {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            kind: Kind::Local,
            host_id: String::new(),
            bind_address: "127.0.0.1".into(),
            bind_port: 0,
            destination_host: String::new(),
            destination_port: 0,
            auto_start: false,
        }
    }
}
impl Forward {
    pub fn validate(&mut self, existing: &[Forward]) -> Result<()> {
        ensure!(existing
                .iter()
                .any(|rule| !self.id.is_empty() && rule.id == self.id)
                || existing.len() < MAX_RULES, "{}", crate::tf!("At most {MAX_RULES} forwarding rules are allowed", "MAX_RULES" => format!("{}", MAX_RULES)));
        self.label = self.label.trim().into();
        self.bind_address = self.bind_address.trim().into();
        self.destination_host = self.destination_host.trim().into();
        ensure!(!self.label.is_empty()
                && self.label.chars().count() <= 128
                && !self.label.chars().any(char::is_control), "{}", crate::i18n::t("Enter a valid rule name (up to 128 characters)"));
        ensure!(!self.host_id.is_empty(), "{}", crate::i18n::t("Select a saved SSH host"));
        ensure!(self.bind_address.parse::<IpAddr>().is_ok(), "{}", crate::i18n::t("Bind address must be an IPv4 or IPv6 address"));
        ensure!(self.bind_port > 0, "{}", crate::i18n::t("Bind port must be between 1 and 65535"));
        if self.kind != Kind::Dynamic {
            ensure!(!self.destination_host.is_empty()
                    && self.destination_host.len() <= 253
                    && !self
                        .destination_host
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control()), "{}", crate::i18n::t("Enter a valid destination host"));
            ensure!(self.destination_port > 0, "{}", crate::i18n::t("Destination port must be between 1 and 65535"));
        }
        ensure!(!existing
                .iter()
                .any(|f| (self.id.is_empty() || f.id != self.id)
                    && f.label.to_lowercase() == self.label.to_lowercase()), "{}", crate::i18n::t("A rule with this name already exists"));
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Stopped,
    Starting,
    Running,
    Error,
}
#[derive(Clone, Debug)]
pub struct Status {
    pub phase: Phase,
    pub message: String,
    pub active_connections: usize,
}
fn publish(tx: &watch::Sender<Status>, phase: Phase, message: impl Into<String>, count: usize) {
    tx.send_replace(Status {
        phase,
        message: message.into(),
        active_connections: count,
    });
}

/// Drop requests cancellation; `stop` additionally waits until all resources are released.
pub struct RunningRule {
    cancel: watch::Sender<bool>,
    status: watch::Receiver<Status>,
    task: Option<JoinHandle<()>>,
}
impl RunningRule {
    pub fn start(
        rt: &Handle,
        mut rule: Forward,
        profile: Profile,
        secret: Zeroizing<String>,
        key_material: Option<crate::keychain::KeyMaterial>,
        known: String,
    ) -> Result<Self> {
        rule.validate(&[])?;
        ensure!(rule.host_id == profile.id, "{}", crate::i18n::t("Forwarding host does not match the saved host"));
        ensure!(!known.is_empty(), "{}", crate::i18n::t("Connect to this host once and trust its fingerprint first"));
        let (cancel, rx) = watch::channel(false);
        let (status_tx, status) = watch::channel(Status {
            phase: Phase::Starting,
            message: crate::i18n::t("Establishing SSH tunnel…").into(),
            active_connections: 0,
        });
        let task = rt.spawn(async move {
            let result = run(
                rule,
                profile,
                secret,
                key_material,
                known,
                rx.clone(),
                &status_tx,
            )
            .await;
            if *rx.borrow() {
                publish(&status_tx, Phase::Stopped, crate::i18n::t("Stopped"), 0);
                return;
            }
            match result {
                Ok(()) => publish(&status_tx, Phase::Stopped, crate::i18n::t("Stopped"), 0),
                Err(error) => publish(&status_tx, Phase::Error, format!("{error:#}"), 0),
            }
        });
        Ok(Self {
            cancel,
            status,
            task: Some(task),
        })
    }
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    pub fn request_stop(&self) {
        self.cancel.send_replace(true);
    }
    pub async fn stop(&mut self) {
        self.request_stop();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for RunningRule {
    fn drop(&mut self) {
        self.request_stop();
    }
}
async fn cancelled(rx: &mut watch::Receiver<bool>) {
    if !*rx.borrow() {
        let _ = rx.changed().await;
    }
}

async fn run(
    rule: Forward,
    profile: Profile,
    secret: Zeroizing<String>,
    key_material: Option<crate::keychain::KeyMaterial>,
    known: String,
    mut cancel: watch::Receiver<bool>,
    status: &watch::Sender<Status>,
) -> Result<()> {
    let (incoming, mut forwarded) = mpsc::channel(MAX_CONNECTIONS);
    let target = (rule.kind == Kind::Remote).then(|| ssh::ForwardTarget {
        address: rule.bind_address.clone(),
        port: rule.bind_port.into(),
        incoming,
    });
    let ssh_cancel = cancel.clone();
    let ssh = tokio::select! {
        _ = cancelled(&mut cancel) => return Ok(()),
        result = timeout(CONNECT_TIMEOUT, ssh::connect_forwarding(&profile, &secret, key_material, known, ssh_cancel, target)) => Arc::new(result.context(crate::i18n::t("SSH tunnel connection timed out"))?.context(crate::i18n::t("Forwarding connection failed; connect to this host once and save working credentials"))?),
    };
    drop(secret);
    let mut jobs = JoinSet::new();
    let result = async {
        let listener = if rule.kind == Kind::Remote {
            tokio::select! {
                _ = cancelled(&mut cancel) => return Ok(()),
                result = timeout(CONNECT_TIMEOUT, ssh.tcpip_forward(&rule.bind_address, rule.bind_port.into())) => { result.context(crate::i18n::t("Remote forwarding request timed out"))?.context(crate::i18n::t("Server rejected remote forwarding"))?; }
            }
            None
        } else {
            Some(TcpListener::bind((rule.bind_address.as_str(), rule.bind_port)).await.context(crate::i18n::t("Cannot listen on the bind address and port"))?)
        };
        publish(status, Phase::Running, crate::i18n::t("Tunnel started"), 0);
        let mut tick = interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = cancelled(&mut cancel) => break,
                _ = tick.tick() => ensure!(!ssh.is_closed(), "{}", crate::i18n::t("SSH tunnel disconnected")),
                completed = jobs.join_next(), if !jobs.is_empty() => {
                    let message = match completed { Some(Ok(Err(e))) => crate::tf!("Connection failed: {e:#}", "e:#" => format!("{:#}", e)), _ => crate::i18n::t("Tunnel started").into() };
                    publish(status, Phase::Running, message, jobs.len());
                }
                accepted = async { listener.as_ref().unwrap().accept().await }, if listener.is_some() => {
                    let (stream, origin) = accepted.context(crate::i18n::t("Listener accept failed"))?;
                    if jobs.len() >= MAX_CONNECTIONS { drop(stream); continue; }
                    let ssh = ssh.clone(); let rule = rule.clone();
                    jobs.spawn(async move { direct(stream, origin, ssh, rule).await });
                    publish(status, Phase::Running, crate::i18n::t("Tunnel started"), jobs.len());
                }
                item = forwarded.recv(), if rule.kind == Kind::Remote => {
                    if let Some(item) = item {
                        if jobs.len() >= MAX_CONNECTIONS { drop(item); continue; }
                        let host = rule.destination_host.clone(); let port = rule.destination_port;
                        jobs.spawn(async move {
                            let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port))).await.context(crate::i18n::t("Destination connection timed out"))??;
                            item.reply.accept().await;
                            let mut tunnel = item.channel.into_stream();
                            copy_bidirectional(&mut stream, &mut tunnel).await?;
                            Ok::<(), anyhow::Error>(())
                        });
                        publish(status, Phase::Running, crate::i18n::t("Tunnel started"), jobs.len());
                    }
                }
            }
        }
        Ok(())
    }.await;
    // Every exit path waits for child tasks before releasing the SSH connection.
    forwarded.close();
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    drop(forwarded);
    if rule.kind == Kind::Remote {
        let _ = timeout(
            Duration::from_secs(3),
            ssh.cancel_tcpip_forward(&rule.bind_address, rule.bind_port.into()),
        )
        .await;
    }
    let _ = timeout(
        Duration::from_secs(3),
        ssh.disconnect(russh::Disconnect::ByApplication, crate::i18n::t("Forwarding stopped"), "en"),
    )
    .await;
    result
}

async fn direct(
    mut stream: TcpStream,
    origin: std::net::SocketAddr,
    ssh: Arc<russh::client::Handle<ssh::Client>>,
    rule: Forward,
) -> Result<()> {
    let dynamic = rule.kind == Kind::Dynamic;
    let (host, port) = if dynamic {
        timeout(CONNECT_TIMEOUT, socks5_connect(&mut stream))
            .await
            .context(crate::i18n::t("SOCKS5 handshake timed out"))??
    } else {
        (rule.destination_host, rule.destination_port)
    };
    let opened = timeout(
        CONNECT_TIMEOUT,
        ssh.channel_open_direct_tcpip(
            host,
            port.into(),
            origin.ip().to_string(),
            origin.port().into(),
        ),
    )
    .await;
    let channel = match opened {
        Ok(Ok(channel)) => channel,
        error => {
            if dynamic {
                let _ = socks_reply(&mut stream, 5).await;
            }
            bail!("{}", crate::tf!("Cannot open SSH forwarding channel: {error:?}", "error:?" => format!("{:?}", error)));
        }
    };
    if dynamic {
        socks_reply(&mut stream, 0).await?;
    }
    let mut tunnel = channel.into_stream();
    copy_bidirectional(&mut stream, &mut tunnel).await?;
    Ok(())
}

async fn socks_reply<S: AsyncWrite + Unpin>(stream: &mut S, code: u8) -> Result<()> {
    stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    Ok(())
}
/// Negotiate no-auth SOCKS5 and parse CONNECT. Caller replies after SSH opens.
pub async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<(String, u16)> {
    let version = stream.read_u8().await?;
    ensure!(version == 5, "{}", crate::i18n::t("Unsupported SOCKS version"));
    let count = stream.read_u8().await? as usize;
    let mut methods = vec![0; count];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        stream.write_all(&[5, 255]).await?;
        bail!("{}", crate::i18n::t("SOCKS5 client does not support authentication-free access"));
    }
    stream.write_all(&[5, 0]).await?;
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[2] != 0 {
        socks_reply(stream, 1).await?;
        bail!("{}", crate::i18n::t("Invalid SOCKS5 request"));
    }
    if header[1] != 1 {
        socks_reply(stream, 7).await?;
        bail!("{}", crate::i18n::t("SOCKS5 supports CONNECT only"));
    }
    let host = match header[3] {
        1 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            std::net::Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            std::net::Ipv6Addr::from(bytes).to_string()
        }
        3 => {
            let size = stream.read_u8().await? as usize;
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).await?;
            match String::from_utf8(bytes) {
                Ok(host)
                    if !host.is_empty()
                        && !host.chars().any(|c| c.is_control() || c.is_whitespace()) =>
                {
                    host
                }
                _ => {
                    socks_reply(stream, 4).await?;
                    bail!("{}", crate::i18n::t("Invalid SOCKS5 domain name"));
                }
            }
        }
        _ => {
            socks_reply(stream, 8).await?;
            bail!("{}", crate::i18n::t("Unsupported SOCKS5 address type"));
        }
    };
    let port = stream.read_u16().await?;
    if port == 0 {
        socks_reply(stream, 1).await?;
        bail!("{}", crate::i18n::t("Invalid SOCKS5 destination port"));
    }
    Ok((host, port))
}
