//! Bounded, asynchronous SSH transport. Network bytes are parsed before a lossy
//! screen notification; UI congestion can skip a frame, never terminal data.
use crate::{
    store::Profile,
    terminal::{Snapshot, Terminal},
};
use anyhow::{Context, Result, bail, ensure};
use russh::{ChannelMsg, Disconnect, client};
use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};
use zeroize::Zeroizing;

#[derive(Debug)]
pub enum Command {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Scroll(i32),
    List(String),
    Upload {
        directory: String,
        local: PathBuf,
        overwrite: bool,
    },
    Download {
        remote: String,
        local: PathBuf,
    },
    Disconnect,
    CreateDir {
        directory: String,
        name: String,
    },
    Rename {
        directory: String,
        from: String,
        to: String,
    },
    Delete {
        directory: String,
        name: String,
        confirmed: bool,
    },
    UploadFile {
        transfer: u64,
        directory: String,
        local: PathBuf,
        overwrite: bool,
    },
    DownloadFile {
        transfer: u64,
        remote: String,
        local: PathBuf,
    },
    UploadTree {
        transfer: u64,
        directory: String,
        local: PathBuf,
        overwrite: bool,
    },
    DownloadTree {
        transfer: u64,
        remote: String,
        local_dir: PathBuf,
    },
    CancelTransfer {
        transfer: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: String,
    pub permissions: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub modified: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Date,
    Size,
    Kind,
}
impl SortColumn {
    pub fn parse(value: &str) -> Self {
        match value {
            "date" | "modified" => Self::Date,
            "size" => Self::Size,
            "kind" => Self::Kind,
            _ => Self::Name,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeSummary {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}
#[derive(Debug)]
pub enum Event {
    Status {
        id: String,
        connected: bool,
        message: String,
        error: bool,
    },
    Screen {
        id: String,
        snapshot: Snapshot,
    },
    HostKey {
        id: String,
        host: String,
        port: u16,
        fingerprint: String,
        reply: oneshot::Sender<bool>,
    },
    AuthPrompt {
        id: String,
        prompts: Vec<(String, bool)>,
        reply: oneshot::Sender<Option<Vec<String>>>,
    },
    Files {
        id: String,
        path: String,
        entries: Vec<RemoteFile>,
    },
    Transfer {
        id: String,
        message: String,
        error: bool,
    },
    OverwriteRequired {
        id: String,
        directory: String,
        local: PathBuf,
        transfer: u64,
        folder: bool,
    },
    DeleteRequired {
        id: String,
        directory: String,
        name: String,
        files: u64,
        dirs: u64,
        bytes: u64,
    },
    TransferProgress {
        id: String,
        transfer: u64,
        name: String,
        direction: String,
        transferred: u64,
        total: u64,
        speed: u64,
    },
    TransferFinished {
        id: String,
        transfer: u64,
        message: String,
        error: bool,
        cancelled: bool,
    },
    Closed {
        id: String,
    },
}
pub struct Session {
    pub id: String,
    pub commands: mpsc::Sender<Command>,
    pub task: JoinHandle<()>,
}

pub fn spawn(
    rt: &Handle,
    id: String,
    profile: Profile,
    secret: String,
    known_key: Option<String>,
    events: mpsc::Sender<Event>,
) -> Session {
    let (commands, rx) = mpsc::channel(64);
    let session_id = id.clone();
    let task = rt.spawn(async move {
        let result = run(
            &id,
            profile,
            Zeroizing::new(secret),
            known_key,
            events.clone(),
            rx,
        )
        .await;
        if let Err(e) = result {
            let _ = events
                .send(Event::Status {
                    id: id.clone(),
                    connected: false,
                    message: format!("{e:#}"),
                    error: true,
                })
                .await;
        }
        let _ = events.send(Event::Closed { id }).await;
    });
    Session {
        id: session_id,
        commands,
        task,
    }
}

struct Client {
    id: String,
    host: String,
    port: u16,
    known: Option<String>,
    events: mpsc::Sender<Event>,
    cancelled: watch::Receiver<bool>,
}
impl client::Handler for Client {
    type Error = anyhow::Error;
    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool> {
        let fingerprint = key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        if let Some(known) = &self.known {
            ensure!(
                known == &fingerprint,
                "SSH 主机密钥已变化，连接已拒绝。请先核实服务器身份。"
            );
            return Ok(true);
        }
        let (tx, rx) = oneshot::channel();
        self.events
            .send(Event::HostKey {
                id: self.id.clone(),
                host: self.host.clone(),
                port: self.port,
                fingerprint,
                reply: tx,
            })
            .await
            .context("界面已关闭")?;
        let trusted = tokio::select! {
            result=timeout(Duration::from_secs(120), rx)=>result.context("主机密钥确认超时")?.unwrap_or(false),
            _=self.cancelled.changed()=>bail!("SSH 连接已取消"),
        };
        ensure!(trusted, "未信任 SSH 主机密钥");
        Ok(true)
    }
}

async fn connect(
    id: &str,
    p: &Profile,
    secret: &str,
    known: Option<String>,
    events: &mpsc::Sender<Event>,
    cancelled: watch::Receiver<bool>,
) -> Result<client::Handle<Client>> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        nodelay: true,
        channel_buffer_size: 32,
        ..Default::default()
    });
    let handler = Client {
        id: id.into(),
        host: p.host.clone(),
        port: p.port,
        known,
        events: events.clone(),
        cancelled,
    };
    let mut ssh = timeout(
        Duration::from_secs(150),
        client::connect(config, (p.host.as_str(), p.port), handler),
    )
    .await
    .context("SSH 连接超时")??;
    let authenticated = if p.auth_type == "key" {
        let path = p.private_key_path.clone();
        let passphrase = Zeroizing::new(secret.to_owned());
        let key = tokio::task::spawn_blocking(move || -> Result<_> {
            ensure!(
                std::fs::metadata(&path)?.len() <= 1024 * 1024,
                "私钥不能超过 1 MB"
            );
            let text = Zeroizing::new(std::fs::read_to_string(&path).context("读取私钥失败")?);
            Ok(russh::keys::decode_secret_key(
                &text,
                if passphrase.is_empty() {
                    None
                } else {
                    Some(passphrase.as_str())
                },
            )
            .context("私钥或口令无效")?)
        })
        .await??;
        let hash = timeout(Duration::from_secs(30), ssh.best_supported_rsa_hash())
            .await
            .context("密钥协商超时")??
            .flatten();
        timeout(
            Duration::from_secs(30),
            ssh.authenticate_publickey(
                p.username.clone(),
                russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            ),
        )
        .await
        .context("私钥认证超时")??
        .success()
    } else {
        timeout(
            Duration::from_secs(30),
            ssh.authenticate_password(p.username.clone(), secret),
        )
        .await
        .context("密码认证超时")??
        .success()
    };
    if !authenticated {
        let mut response = timeout(
            Duration::from_secs(30),
            ssh.authenticate_keyboard_interactive_start(p.username.clone(), None::<String>),
        )
        .await
        .context("交互认证超时")??;
        let mut rounds = 0;
        loop {
            match response {
                client::KeyboardInteractiveAuthResponse::Success => break,
                client::KeyboardInteractiveAuthResponse::Failure { .. } => {
                    bail!("SSH 认证失败，请检查用户名和凭据")
                }
                client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    rounds += 1;
                    ensure!(rounds <= 8, "交互认证轮次过多");
                    let (tx, rx) = oneshot::channel();
                    events
                        .send(Event::AuthPrompt {
                            id: id.into(),
                            prompts: prompts.into_iter().map(|p| (p.prompt, p.echo)).collect(),
                            reply: tx,
                        })
                        .await
                        .context("界面已关闭")?;
                    let answers = timeout(Duration::from_secs(120), rx)
                        .await
                        .context("交互认证确认超时")??
                        .context("认证已取消")?;
                    response = timeout(
                        Duration::from_secs(30),
                        ssh.authenticate_keyboard_interactive_respond(answers),
                    )
                    .await
                    .context("交互认证超时")??;
                }
            }
        }
    }
    Ok(ssh)
}

// Cancelling the owning session also cancels the SFTP task and closes its channels.
struct AbortOnDrop(JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort()
    }
}
struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}
enum Write {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Pty,
    Shell,
}
async fn run(
    id: &str,
    p: Profile,
    secret: Zeroizing<String>,
    known: Option<String>,
    events: mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
) -> Result<()> {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let _cancellation = CancelOnDrop(cancel_tx);
    let ssh = {
        let connecting = connect(id, &p, &secret, known, &events, cancel_rx);
        tokio::pin!(connecting);
        loop {
            tokio::select! {
                result=&mut connecting=>break Arc::new(result?),
                command=commands.recv()=>if matches!(command,None|Some(Command::Disconnect)){return Ok(())}
            }
        }
    };
    drop(secret);
    let opening = timeout(Duration::from_secs(30), ssh.channel_open_session());
    tokio::pin!(opening);
    let channel = loop {
        tokio::select! {
            result=&mut opening=>break result.context("终端通道超时")??,
            command=commands.recv()=>if matches!(command,None|Some(Command::Disconnect)){return Ok(())}
        }
    };
    let (mut reader, writer) = channel.split();
    let (write_tx, mut write_rx) = mpsc::channel(8);
    let writing = async move {
        while let Some(command) = write_rx.recv().await {
            match command {
                Write::Input(data) => writer.data(data.as_slice()).await?,
                Write::Resize { cols, rows } => {
                    writer.window_change(cols.into(), rows.into(), 0, 0).await?
                }
                Write::Pty => {
                    writer
                        .request_pty(true, "xterm-256color", 100, 30, 0, 0, &[])
                        .await?
                }
                Write::Shell => writer.request_shell(true).await?,
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(writing);
    let mut pending_write = None;
    let mut pending_control = Some(Write::Pty);
    let mut protocol = Vec::new();
    let mut terminal = Terminal::new(30, 100);
    let mut stage = 0;
    let mut connected = false;
    let mut setup_timeout = Box::pin(tokio::time::sleep(Duration::from_secs(30)));
    let mut dirty = true;
    let mut frame = interval(Duration::from_millis(34));
    frame.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let (job_tx, job_rx) = mpsc::channel(8);
    let cancelled: CancelledSet = Arc::new(StdMutex::new(HashSet::new()));
    let _worker = AbortOnDrop(tokio::spawn(sftp_worker(
        id.into(),
        ssh.clone(),
        events.clone(),
        job_rx,
        cancelled.clone(),
    )));
    loop {
        tokio::select! {
            result=&mut writing=>return result,
            _=&mut setup_timeout,if !connected=>bail!("终端请求超时"),
            permit=write_tx.reserve(),if pending_write.is_some()||pending_control.is_some()||!protocol.is_empty()=>{
                let next=if let Some(control)=pending_control.take(){control}
                    else if !protocol.is_empty(){Write::Input(std::mem::take(&mut protocol))}
                    else{pending_write.take().context("缺少终端输入")?};
                permit.context("终端输入通道已关闭")?.send(next);
            },
            command=commands.recv(),if pending_write.is_none()=>match command {
                None|Some(Command::Disconnect)=>break,
                Some(_) if !connected=>{},
                Some(Command::Input(data))=>{pending_write=Some(Write::Input(data));},
                Some(Command::Resize{cols,rows})=>{let cols=cols.clamp(2,500);let rows=rows.clamp(2,300);pending_write=Some(Write::Resize{cols,rows});terminal.resize(rows,cols);dirty=true;},
                Some(Command::Scroll(delta))=>{terminal.scroll(delta);dirty=true;},
                Some(Command::CancelTransfer{transfer})=>{
                    if let Ok(mut pending)=cancelled.lock() {
                        if pending.len()<1024 {pending.insert(transfer);}
                    }
                },
                Some(job)=>if let Err(error)=job_tx.try_send(job){
                    let message = "文件任务队列繁忙，请稍后重试".to_string();
                    if let Some(transfer) = transfer_id(error.into_inner()) {
                        complete_transfer(&events, &cancelled, id, transfer, Err(anyhow::anyhow!(message))).await?;
                    } else {
                        events.send(Event::Transfer{id:id.into(),message,error:true}).await.context("界面已关闭")?;
                    }
                }
            },
            message=reader.wait()=>match message {
                Some(ChannelMsg::Data{data})|Some(ChannelMsg::ExtendedData{data,..})=>{
                    terminal.feed(&data);
                    let responses=terminal.take_responses();
                    ensure!(protocol.len()+responses.len()<=65536,"服务器终端查询过于频繁");
                    protocol.extend(responses);dirty=true;
                },
                Some(ChannelMsg::Success) if !connected=>{
                    if stage==0 {stage=1;pending_control=Some(Write::Shell);setup_timeout.as_mut().reset(tokio::time::Instant::now()+Duration::from_secs(30));}
                    else {connected=true;events.send(Event::Status{id:id.into(),connected:true,message:"已连接".into(),error:false}).await.context("界面已关闭")?;}
                },
                None|Some(ChannelMsg::Close)=>break,
                Some(ChannelMsg::Failure)=>bail!("服务器拒绝终端请求"),
                _=>{}
            },
            _=frame.tick(),if dirty&&connected=>{
                match events.try_send(Event::Screen{id:id.into(),snapshot:terminal.snapshot()}) {
                    Ok(())=>dirty=false,
                    Err(mpsc::error::TrySendError::Full(_))=>{},
                    Err(mpsc::error::TrySendError::Closed(_))=>break,
                }
            }
        }
    }
    if dirty {
        let _ = events.try_send(Event::Screen {
            id: id.into(),
            snapshot: terminal.snapshot(),
        });
    }
    let _ = timeout(
        Duration::from_secs(2),
        ssh.disconnect(Disconnect::ByApplication, "Disconnected", "en"),
    )
    .await;
    Ok(())
}

fn validate_remote(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 32768 && !path.chars().any(char::is_control),
        "远程路径无效"
    );
    Ok(())
}
/// One SFTP path component: no separators, no relative traversal, no control bytes.
pub fn safe_remote_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}
/// A name that may be joined to a local directory without escaping it.
/// Rejects Windows separators, drive/ADS colons, traversal and device names.
pub fn safe_local_component(name: &str) -> bool {
    if !safe_remote_component(name)
        || name.contains([':', '<', '>', '"', '|', '?', '*'])
        || name.ends_with(['.', ' '])
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ').to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}
fn remote_join(directory: &str, name: &str) -> Result<String> {
    validate_remote(directory)?;
    ensure!(safe_remote_component(name), "文件名无效");
    Ok(format!("{}/{}", directory.trim_end_matches('/'), name))
}
fn remote_child(root: &str, relative: &str) -> Result<String> {
    let mut path = root.trim_end_matches('/').to_string();
    for component in relative.split('/') {
        ensure!(safe_remote_component(component), "远程文件名无效");
        path.push('/');
        path.push_str(component);
    }
    Ok(path)
}
fn local_child(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        ensure!(safe_local_component(component), "本地文件名无效");
        path.push(component);
    }
    Ok(path)
}
/// Render Unix mode bits (low nine bits) as `rwxr-xr-x`.
pub fn format_permissions(mode: u32) -> String {
    let letters = [b'r', b'w', b'x'];
    (0..9)
        .map(|bit| {
            if mode & (1 << (8 - bit)) != 0 {
                letters[bit % 3] as char
            } else {
                '-'
            }
        })
        .collect()
}
fn modified_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}
fn sort_entries<T>(
    entries: &mut [T],
    column: SortColumn,
    ascending: bool,
    fields: impl Fn(&T) -> (&str, bool, u64, i64),
) {
    entries.sort_by(|a, b| {
        let (a_name, a_dir, a_size, a_modified) = fields(a);
        let (b_name, b_dir, b_size, b_modified) = fields(b);
        let order = match column {
            SortColumn::Name => a_name.to_lowercase().cmp(&b_name.to_lowercase()),
            SortColumn::Date => a_modified.cmp(&b_modified),
            SortColumn::Size => a_size.cmp(&b_size),
            SortColumn::Kind => {
                let kind = |name: &str| {
                    Path::new(name)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_lowercase()
                };
                kind(a_name).cmp(&kind(b_name))
            }
        };
        b_dir.cmp(&a_dir).then_with(|| {
            let order = if ascending { order } else { order.reverse() };
            order.then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase()))
        })
    });
}
pub fn sort_remote_files(entries: &mut [RemoteFile], column: SortColumn, ascending: bool) {
    sort_entries(entries, column, ascending, |entry| {
        (
            &entry.name,
            entry.is_dir,
            entry.size,
            entry.modified.parse().unwrap_or(0),
        )
    })
}
pub fn sort_local_files(entries: &mut [LocalFile], column: SortColumn, ascending: bool) {
    sort_entries(entries, column, ascending, |entry| {
        (&entry.name, entry.is_dir, entry.size, entry.modified)
    })
}
pub fn read_local_dir(path: &Path) -> Result<Vec<LocalFile>> {
    let path = path.canonicalize()?;
    ensure!(path.is_dir(), "请选择本地文件夹");
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path)?.take(20000) {
        let entry = entry?;
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if name.chars().any(char::is_control) {
            continue;
        }
        entries.push(LocalFile {
            name,
            path: entry.path(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            is_dir: meta.is_dir(),
            modified: meta.modified().map(modified_seconds).unwrap_or(0),
        });
    }
    Ok(entries)
}
pub fn local_tree_summary(root: &Path) -> Result<TreeSummary> {
    let mut summary = TreeSummary::default();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0u64;
    while let Some(path) = stack.pop() {
        let meta = std::fs::symlink_metadata(&path)?;
        if local_is_link(&meta) {
            summary.files += 1;
            continue;
        }
        if meta.is_dir() {
            if path != root {
                summary.dirs += 1;
            }
            for entry in std::fs::read_dir(&path)? {
                seen += 1;
                ensure!(seen <= 100_000, "目录项目过多，请缩小操作范围");
                stack.push(entry?.path());
            }
        } else {
            summary.files += 1;
            summary.bytes = summary.bytes.saturating_add(meta.len());
        }
    }
    Ok(summary)
}
pub fn delete_local_tree(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() && !local_is_link(&meta) {
        for entry in std::fs::read_dir(path)? {
            delete_local_tree(&entry?.path())?;
        }
        std::fs::remove_dir(path)?;
    } else if meta.is_dir() {
        std::fs::remove_dir(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn local_is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn ensure_local_directory(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !local_is_link(&meta),
        "本地目标不是常规文件夹，不能通过链接写入"
    );
    Ok(())
}
fn transfer_id(command: Command) -> Option<u64> {
    match command {
        Command::UploadFile { transfer, .. }
        | Command::DownloadFile { transfer, .. }
        | Command::UploadTree { transfer, .. }
        | Command::DownloadTree { transfer, .. } => Some(transfer),
        _ => None,
    }
}
type CancelledSet = Arc<StdMutex<HashSet<u64>>>;

#[derive(Debug)]
struct TransferCancelled;
impl std::fmt::Display for TransferCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("传输已取消")
    }
}
impl std::error::Error for TransferCancelled {}
fn ensure_active(cancelled: &CancelledSet, transfer: u64) -> Result<()> {
    if cancelled
        .lock()
        .map(|pending| pending.contains(&transfer))
        .unwrap_or(false)
    {
        return Err(anyhow::Error::new(TransferCancelled));
    }
    Ok(())
}
fn clear_cancelled(cancelled: &CancelledSet, transfer: u64) {
    if let Ok(mut pending) = cancelled.lock() {
        pending.remove(&transfer);
    }
}
struct Progress<'a> {
    session: &'a str,
    transfer: u64,
    name: &'a str,
    direction: &'a str,
    events: &'a mpsc::Sender<Event>,
    cancelled: &'a CancelledSet,
    total: u64,
    done: u64,
    last_emit: Instant,
    last_done: u64,
    last_time: Instant,
}
impl<'a> Progress<'a> {
    fn new(
        session: &'a str,
        transfer: u64,
        name: &'a str,
        direction: &'a str,
        events: &'a mpsc::Sender<Event>,
        cancelled: &'a CancelledSet,
        total: u64,
    ) -> Self {
        let now = Instant::now();
        Self {
            session,
            transfer,
            name,
            direction,
            events,
            cancelled,
            total,
            done: 0,
            last_emit: now,
            last_done: 0,
            last_time: now,
        }
    }
    async fn advance(&mut self, bytes: u64) -> Result<()> {
        self.done = self.done.saturating_add(bytes);
        ensure_active(self.cancelled, self.transfer)?;
        self.emit(false).await
    }
    async fn emit(&mut self, force: bool) -> Result<()> {
        let now = Instant::now();
        if !force && now.duration_since(self.last_emit) < Duration::from_millis(120) {
            return Ok(());
        }
        let seconds = now.duration_since(self.last_time).as_secs_f64().max(0.001);
        let speed = ((self.done.saturating_sub(self.last_done)) as f64 / seconds) as u64;
        self.last_done = self.done;
        self.last_time = now;
        self.last_emit = now;
        self.events
            .send(Event::TransferProgress {
                id: self.session.into(),
                transfer: self.transfer,
                name: self.name.into(),
                direction: self.direction.into(),
                transferred: self.done,
                total: self.total,
                speed,
            })
            .await
            .context("界面已关闭")
    }
    async fn finish_progress(&mut self) -> Result<()> {
        self.emit(true).await
    }
}
async fn copy_with_progress<R, W>(
    input: &mut R,
    output: &mut W,
    progress: &mut Progress<'_>,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; 64 * 1024];
    let mut copied = 0u64;
    loop {
        ensure_active(progress.cancelled, progress.transfer)?;
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).await?;
        copied = copied.saturating_add(read as u64);
        progress.advance(read as u64).await?;
    }
    Ok(copied)
}
async fn complete_transfer(
    events: &mpsc::Sender<Event>,
    cancelled: &CancelledSet,
    session: &str,
    transfer: u64,
    result: Result<String>,
) -> Result<()> {
    clear_cancelled(cancelled, transfer);
    let (message, error, was_cancelled) = match result {
        Ok(message) => (message, false, false),
        Err(error) if error.downcast_ref::<TransferCancelled>().is_some() => {
            ("传输已取消".to_string(), false, true)
        }
        Err(error) => (format!("{error:#}"), true, false),
    };
    events
        .send(Event::TransferFinished {
            id: session.into(),
            transfer,
            message,
            error,
            cancelled: was_cancelled,
        })
        .await
        .context("界面已关闭")
}
async fn sftp_worker(
    id: String,
    ssh: Arc<client::Handle<Client>>,
    events: mpsc::Sender<Event>,
    mut jobs: mpsc::Receiver<Command>,
    cancelled: CancelledSet,
) {
    let mut sftp = None;
    while let Some(job) = jobs.recv().await {
        let transfer = match &job {
            Command::UploadFile { transfer, .. } | Command::DownloadFile { transfer, .. }
            | Command::UploadTree { transfer, .. } | Command::DownloadTree { transfer, .. } => Some(*transfer),
            _ => None,
        };
        let result = async {
            if let Some(transfer) = transfer { ensure_active(&cancelled, transfer)?; }
            if sftp.is_none() {
                let channel = timeout(Duration::from_secs(30), ssh.channel_open_session())
                    .await
                    .context("SFTP 通道超时")??;
                channel.request_subsystem(true, "sftp").await?;
                let client = timeout(
                    Duration::from_secs(30),
                    SftpSession::new(channel.into_stream()),
                )
                .await
                .context("SFTP 握手超时")??;
                client.set_timeout(30);
                sftp = Some(client);
            }
            let client = sftp.as_ref().context("SFTP 未初始化")?;
            execute_file_job(&id, client, &events, &cancelled, job).await
        }
        .await;
        if let Err(e) = result {
            if let Some(transfer) = transfer {
                let _ = complete_transfer(&events, &cancelled, &id, transfer, Err(e)).await;
                continue;
            }
            let _ = events
                .send(Event::Transfer {
                    id: id.clone(),
                    message: format!("{e:#}"),
                    error: true,
                })
                .await;
        }
    }
}

struct RemoteEntry {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    is_symlink: bool,
    modified: String,
    permissions: Option<u32>,
}
async fn read_remote_entries(
    sftp: &SftpSession,
    path: &str,
    strict: bool,
) -> Result<Vec<RemoteEntry>> {
    validate_remote(path)?;
    let mut entries = Vec::new();
    for entry in sftp.read_dir(path.to_string()).await? {
        ensure!(entries.len() < 100_000, "目录项目过多，请缩小浏览范围");
        let name = entry.file_name();
        if name == "." || name == ".." { continue; }
        if !safe_remote_component(&name) {
            ensure!(!strict, "远程文件名包含不安全字符：{name:?}");
            continue;
        }
        let meta = entry.metadata();
        entries.push(RemoteEntry {
            path: format!("{}/{}", path.trim_end_matches('/'), name),
            name,
            size: meta.size.unwrap_or(0),
            is_dir: meta.is_dir(),
            is_symlink: meta.is_symlink(),
            modified: meta.mtime.map(|n| n.to_string()).unwrap_or_default(),
            permissions: meta.permissions,
        });
    }
    Ok(entries)
}
async fn send_files(
    id: &str,
    sftp: &SftpSession,
    events: &mpsc::Sender<Event>,
    path: &str,
) -> Result<()> {
    validate_remote(path)?;
    let path = sftp.canonicalize(path).await?;
    let mut entries = read_remote_entries(sftp, &path, false)
        .await?
        .into_iter()
        .map(|entry| RemoteFile {
            name: entry.name,
            path: entry.path,
            size: entry.size,
            is_dir: entry.is_dir,
            modified: entry.modified,
            permissions: entry.permissions,
        })
        .collect::<Vec<_>>();
    sort_remote_files(&mut entries, SortColumn::Name, true);
    events
        .send(Event::Files {
            id: id.into(),
            path,
            entries,
        })
        .await
        .context("界面已关闭")
}
async fn send_message(
    id: &str,
    events: &mpsc::Sender<Event>,
    message: String,
    error: bool,
) -> Result<()> {
    events
        .send(Event::Transfer {
            id: id.into(),
            message,
            error,
        })
        .await
        .context("界面已关闭")
}
struct LocalTree {
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
    total: u64,
}
fn scan_local_tree(root: &Path) -> Result<LocalTree> {
    ensure_local_directory(root)?;
    let mut tree = LocalTree {
        dirs: Vec::new(),
        files: Vec::new(),
        total: 0,
    };
    let mut stack = vec![(root.to_path_buf(), String::new())];
    let mut seen = 0usize;
    while let Some((directory, relative)) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            seen += 1;
            ensure!(seen <= 100_000, "目录项目过多，请缩小上传范围");
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            ensure!(safe_remote_component(&name), "本地文件名无法用于远程路径");
            let relative = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let meta = std::fs::symlink_metadata(entry.path())?;
            if local_is_link(&meta) {
                // Never follow links outside the selected folder.
                continue;
            }
            if meta.is_dir() {
                tree.dirs.push(relative.clone());
                stack.push((entry.path(), relative));
            } else if meta.is_file() {
                tree.files.push((relative, meta.len()));
                tree.total = tree.total.saturating_add(meta.len());
            }
        }
    }
    tree.dirs.sort_by_key(|d| d.matches('/').count());
    tree.files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(tree)
}
struct RemoteTree {
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
    total: u64,
    links: u64,
}
async fn scan_remote_tree(sftp: &SftpSession, root: &str) -> Result<RemoteTree> {
    let mut tree = RemoteTree {
        dirs: Vec::new(),
        files: Vec::new(),
        total: 0,
        links: 0,
    };
    let mut stack = vec![(root.to_string(), String::new())];
    let mut seen = 0usize;
    while let Some((directory, relative)) = stack.pop() {
        for entry in read_remote_entries(sftp, &directory, true).await? {
            seen += 1;
            ensure!(seen <= 100_000, "目录项目过多，请缩小下载范围");
            let child = if relative.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative}/{}", entry.name)
            };
            if entry.is_symlink {
                tree.links += 1;
                continue;
            }
            if entry.is_dir {
                tree.dirs.push(child.clone());
                stack.push((entry.path, child));
            } else {
                tree.files.push((child, entry.size));
                tree.total = tree.total.saturating_add(entry.size);
            }
        }
    }
    tree.dirs.sort_by_key(|d| d.matches('/').count());
    tree.files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(tree)
}
async fn delete_remote_tree(sftp: &SftpSession, path: &str) -> Result<()> {
    let mut stack = vec![(path.to_string(), false)];
    let mut seen = 0usize;
    while let Some((path, visited)) = stack.pop() {
        if visited { sftp.remove_dir(path).await?; continue; }
        seen += 1;
        ensure!(seen <= 100_000, "目录项目过多，请缩小删除范围");
        let meta = sftp.symlink_metadata(path.clone()).await?;
        if meta.is_dir() && !meta.is_symlink() {
            let entries = read_remote_entries(sftp, &path, true).await?;
            stack.push((path, true));
            for entry in entries { stack.push((entry.path, false)); }
        } else {
            sftp.remove_file(path).await?;
        }
    }
    Ok(())
}
async fn upload_file_progress(
    sftp: &SftpSession,
    local: &Path,
    remote: &str,
    overwrite: bool,
    progress: &mut Progress<'_>,
) -> Result<u64> {
    ensure_active(progress.cancelled, progress.transfer)?;
    let mut input = tokio::fs::File::open(local).await?;
    ensure!(input.metadata().await?.is_file(), "只能上传常规文件");
    if overwrite && sftp.try_exists(remote.to_string()).await? {
        let meta = sftp.symlink_metadata(remote.to_string()).await?;
        ensure!(meta.is_regular() && !meta.is_symlink(), "远程目标不是常规文件");
    }
    let flags = OpenFlags::WRITE
        | OpenFlags::CREATE
        | if overwrite {
            OpenFlags::TRUNCATE
        } else {
            OpenFlags::EXCLUDE
        };
    let mut output = sftp.open_with_flags(remote.to_string(), flags).await?;
    let result = copy_with_progress(&mut input, &mut output, progress).await;
    if result.is_err() {
        let _ = output.shutdown().await;
        drop(output);
        if !overwrite { let _ = sftp.remove_file(remote.to_string()).await; }
        return result;
    }
    let size = result?;
    output.shutdown().await?;
    Ok(size)
}
async fn download_file_progress(
    sftp: &SftpSession,
    remote: &str,
    local: &Path,
    progress: &mut Progress<'_>,
) -> Result<u64> {
    validate_remote(remote)?;
    ensure!(local.file_name().and_then(|n| n.to_str()).is_some_and(safe_local_component), "本地文件名无效");
    ensure!(!local.exists(), "本地文件已存在，请选择新的保存名称");
    let parent = local.parent().context("保存路径无效")?;
    let temp = parent.join(format!(".opentermius-{}.part", uuid::Uuid::new_v4()));
    let guard = PartialDownload(temp.clone());
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .await?;
    let mut input = sftp.open(remote.to_string()).await?;
    let size = copy_with_progress(&mut input, &mut output, progress).await?;
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    ensure_active(progress.cancelled, progress.transfer)?;
    let commit_temp = temp.clone();
    let commit_local = local.to_path_buf();
    tokio::task::spawn_blocking(move || commit_download(&commit_temp, &commit_local))
        .await
        .context("下载保存任务失败")?
        .context("保存失败，目标可能已存在或不可写")?;
    drop(guard);
    Ok(size)
}

async fn execute_file_job(
    id: &str,
    sftp: &SftpSession,
    events: &mpsc::Sender<Event>,
    cancelled: &CancelledSet,
    job: Command,
) -> Result<()> {
    match job {
        Command::List(path) => {
            send_files(id, sftp, events, &path).await?;
        }
        Command::Upload {
            directory,
            local,
            overwrite,
        } => {
            let name = local
                .file_name()
                .and_then(|s| s.to_str())
                .context("本地文件名无效")?;
            let remote = remote_join(&directory, name)?;
            if !overwrite && sftp.try_exists(remote.clone()).await? {
                events
                    .send(Event::OverwriteRequired {
                        id: id.into(),
                        directory,
                        local,
                        transfer: 0,
                        folder: false,
                    })
                    .await
                    .context("界面已关闭")?;
                return Ok(());
            }
            let mut input = tokio::fs::File::open(&local).await?;
            ensure!(input.metadata().await?.is_file(), "只能上传常规文件");
            let flags = OpenFlags::WRITE
                | OpenFlags::CREATE
                | if overwrite {
                    OpenFlags::TRUNCATE
                } else {
                    OpenFlags::EXCLUDE
                };
            let mut output = sftp.open_with_flags(remote, flags).await?;
            let size = tokio::io::copy(&mut input, &mut output).await?;
            output.shutdown().await?;
            send_message(id, events, format!("已上传 {name}，{size} 字节"), false).await?;
        }
        Command::Download { remote, local } => {
            validate_remote(&remote)?;
            ensure!(!local.exists(), "本地文件已存在，请选择新的保存名称");
            let parent = local.parent().context("保存路径无效")?;
            let temp = parent.join(format!(".opentermius-{}.part", uuid::Uuid::new_v4()));
            let guard = PartialDownload(temp.clone());
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await?;
            let mut input = sftp.open(remote).await?;
            let size = tokio::io::copy(&mut input, &mut output).await?;
            output.flush().await?;
            output.sync_all().await?;
            drop(output);
            let commit_temp = temp.clone();
            let commit_local = local.clone();
            tokio::task::spawn_blocking(move || commit_download(&commit_temp, &commit_local))
                .await
                .context("下载保存任务失败")?
                .context("保存失败，目标可能已存在或不可写")?;
            drop(guard);
            send_message(
                id,
                events,
                format!("已下载 {}，{size} 字节", local.display()),
                false,
            )
            .await?;
        }
        Command::CreateDir { directory, name } => {
            let path = remote_join(&directory, &name)?;
            ensure!(!sftp.try_exists(path.clone()).await?, "同名文件夹已存在");
            sftp.create_dir(path).await?;
            send_message(id, events, format!("已新建文件夹 {name}"), false).await?;
            send_files(id, sftp, events, &directory).await?;
        }
        Command::Rename {
            directory,
            from,
            to,
        } => {
            let source = remote_join(&directory, &from)?;
            let target = remote_join(&directory, &to)?;
            ensure!(!sftp.try_exists(target.clone()).await?, "目标名称已存在");
            sftp.rename(source, target).await?;
            send_message(id, events, format!("已将 {from} 重命名为 {to}"), false).await?;
            send_files(id, sftp, events, &directory).await?;
        }
        Command::Delete {
            directory,
            name,
            confirmed,
        } => {
            let path = remote_join(&directory, &name)?;
            if !confirmed {
                let meta = sftp.symlink_metadata(path.clone()).await?;
                let (files, dirs, bytes) = if meta.is_dir() && !meta.is_symlink() {
                    let tree = scan_remote_tree(sftp, &path).await?;
                    (
                        tree.files.len() as u64 + tree.links,
                        tree.dirs.len() as u64,
                        tree.total,
                    )
                } else {
                    (1, 0, meta.size.unwrap_or(0))
                };
                events
                    .send(Event::DeleteRequired {
                        id: id.into(),
                        directory,
                        name,
                        files,
                        dirs,
                        bytes,
                    })
                    .await
                    .context("界面已关闭")?;
            } else {
                delete_remote_tree(sftp, &path).await?;
                send_message(id, events, format!("已删除 {name}"), false).await?;
                send_files(id, sftp, events, &directory).await?;
            }
        }
        Command::UploadFile {
            transfer,
            directory,
            local,
            overwrite,
        } => {
            let outcome: Result<Option<String>> = async {
                let name = local
                    .file_name()
                    .and_then(|s| s.to_str())
                    .context("本地文件名无效")?
                    .to_string();
                let remote = remote_join(&directory, &name)?;
                if !overwrite && sftp.try_exists(remote.clone()).await? {
                    events
                        .send(Event::OverwriteRequired {
                            id: id.into(),
                            directory,
                            local,
                            transfer,
                            folder: false,
                        })
                        .await
                        .context("界面已关闭")?;
                    return Ok(None);
                }
                let total = tokio::fs::metadata(&local).await?.len();
                let mut progress =
                    Progress::new(id, transfer, &name, "Upload", events, cancelled, total);
                let size =
                    upload_file_progress(sftp, &local, &remote, overwrite, &mut progress).await?;
                progress.finish_progress().await?;
                Ok(Some(format!("已上传 {name}，{size} 字节")))
            }
            .await;
            match outcome {
                Ok(Some(message)) => {
                    complete_transfer(events, cancelled, id, transfer, Ok(message)).await?
                }
                Ok(None) => {}
                Err(error) => {
                    complete_transfer(events, cancelled, id, transfer, Err(error)).await?
                }
            }
        }
        Command::DownloadFile {
            transfer,
            remote,
            local,
        } => {
            let name = local
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("download")
                .to_string();
            let result = async {
                let total = sftp.metadata(remote.clone()).await?.size.unwrap_or(0);
                let mut progress =
                    Progress::new(id, transfer, &name, "Download", events, cancelled, total);
                let size = download_file_progress(sftp, &remote, &local, &mut progress).await?;
                progress.finish_progress().await?;
                Ok(format!("已下载 {name}，{size} 字节"))
            }
            .await;
            complete_transfer(events, cancelled, id, transfer, result).await?;
        }
        Command::UploadTree {
            transfer,
            directory,
            local,
            overwrite,
        } => {
            let outcome: Result<Option<String>> = async {
                let name = local
                    .file_name()
                    .and_then(|s| s.to_str())
                    .context("本地文件夹名无效")?
                    .to_string();
                let target = remote_join(&directory, &name)?;
                let scan_local = local.clone();
                let tree = tokio::task::spawn_blocking(move || scan_local_tree(&scan_local))
                    .await
                    .context("读取本地文件夹失败")??;
                if !overwrite && sftp.try_exists(target.clone()).await? {
                    events
                        .send(Event::OverwriteRequired {
                            id: id.into(),
                            directory,
                            local,
                            transfer,
                            folder: true,
                        })
                        .await
                        .context("界面已关闭")?;
                    return Ok(None);
                }
                if sftp.try_exists(target.clone()).await? {
                    let meta = sftp.symlink_metadata(target.clone()).await?;
                    ensure!(
                        meta.is_dir() && !meta.is_symlink(),
                        "远程目标不是常规文件夹"
                    );
                } else {
                    sftp.create_dir(target.clone()).await?;
                }
                for directory in &tree.dirs {
                    ensure_active(cancelled, transfer)?;
                    let path = remote_child(&target, directory)?;
                    if !sftp.try_exists(path.clone()).await? {
                        sftp.create_dir(path).await?;
                    } else {
                        let meta = sftp.symlink_metadata(path).await?;
                        ensure!(
                            meta.is_dir() && !meta.is_symlink(),
                            "远程目标不是常规文件夹"
                        );
                    }
                }
                let mut progress =
                    Progress::new(id, transfer, &name, "Upload", events, cancelled, tree.total);
                let mut bytes = 0u64;
                for (relative, _) in &tree.files {
                    let remote = remote_child(&target, relative)?;
                    let local = local_child(&local, relative)?;
                    bytes += upload_file_progress(sftp, &local, &remote, overwrite, &mut progress)
                        .await?;
                }
                progress.finish_progress().await?;
                Ok(Some(format!(
                    "已上传文件夹 {name}，{} 个文件，{bytes} 字节",
                    tree.files.len()
                )))
            }
            .await;
            match outcome {
                Ok(Some(message)) => {
                    complete_transfer(events, cancelled, id, transfer, Ok(message)).await?
                }
                Ok(None) => {}
                Err(error) => {
                    complete_transfer(events, cancelled, id, transfer, Err(error)).await?
                }
            }
        }
        Command::DownloadTree {
            transfer,
            remote,
            local_dir,
        } => {
            let result = async {
                validate_remote(&remote)?;
                let remote = sftp.canonicalize(remote).await?;
                let name = remote
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .context("远程文件夹名无效")?;
                ensure!(safe_local_component(name), "远程文件夹名包含不安全字符");
                let root = local_dir.join(name);
                ensure_local_directory(&local_dir)?;
                if root.try_exists()? {
                    ensure_local_directory(&root)?;
                }
                let tree = scan_remote_tree(sftp, &remote).await?;
                // Validate every name before creating anything, including empty folders.
                for directory in &tree.dirs {
                    local_child(&root, directory)?;
                }
                for (relative, _) in &tree.files {
                    ensure!(
                        !local_child(&root, relative)?.exists(),
                        "本地文件已存在，请选择新的保存位置"
                    );
                }
                ensure_active(cancelled, transfer)?;
                if !root.try_exists()? {
                    tokio::fs::create_dir(&root).await?;
                }
                for directory in &tree.dirs {
                    ensure_active(cancelled, transfer)?;
                    let path = local_child(&root, directory)?;
                    if path.try_exists()? {
                        ensure_local_directory(&path)?;
                    } else {
                        tokio::fs::create_dir(&path).await?;
                    }
                }
                let mut progress = Progress::new(
                    id, transfer, name, "Download", events, cancelled, tree.total,
                );
                let mut bytes = 0u64;
                for (relative, _) in &tree.files {
                    let remote = remote_child(&remote, relative)?;
                    let local = local_child(&root, relative)?;
                    bytes += download_file_progress(sftp, &remote, &local, &mut progress).await?;
                }
                progress.finish_progress().await?;
                Ok(format!(
                    "已下载文件夹 {name}，{} 个文件，{bytes} 字节",
                    tree.files.len()
                ))
            }
            .await;
            complete_transfer(events, cancelled, id, transfer, result).await?;
        }
        _ => {}
    }
    Ok(())
}
struct PartialDownload(PathBuf);
impl Drop for PartialDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Commit a same-directory temporary download atomically, without replacement.
/// Windows MoveFileW also works on FAT/exFAT and shares without hard-link support.
#[cfg(windows)]
fn commit_download(temp: &std::path::Path, local: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // Canonical parents supply absolute verbatim paths for long-path support.
    let temp = std::fs::canonicalize(temp)?;
    let name = local.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid download filename",
        )
    })?;
    let parent = local.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid download directory",
        )
    })?;
    let local = std::fs::canonicalize(parent)?.join(name);
    let mut source: Vec<u16> = temp.as_os_str().encode_wide().collect();
    let mut destination: Vec<u16> = local.as_os_str().encode_wide().collect();
    if source.contains(&0) || destination.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "download path contains NUL",
        ));
    }
    source.push(0);
    destination.push(0);
    // MoveFileW intentionally has no replace-existing flag: a destination
    // created after the initial existence check remains untouched.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), destination.as_ptr())
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn commit_download(temp: &std::path::Path, local: &std::path::Path) -> std::io::Result<()> {
    std::fs::hard_link(temp, local)
}

#[cfg(all(test, windows))]
mod download_tests {
    #[test]
    fn atomic_download_commit_never_replaces_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("incoming.part");
        let destination = dir.path().join("saved.bin");
        std::fs::write(&temp, b"incoming").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        assert!(super::commit_download(&temp, &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert_eq!(std::fs::read(&temp).unwrap(), b"incoming");
        std::fs::remove_file(&destination).unwrap();
        super::commit_download(&temp, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"incoming");
        assert!(!temp.exists());
    }
}
