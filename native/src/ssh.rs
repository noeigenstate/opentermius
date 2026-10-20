//! Bounded, asynchronous SSH transport. Network bytes are parsed before a lossy
//! screen notification; UI congestion can skip a frame, never terminal data.
use crate::{
    store::Profile,
    terminal::{Snapshot, Terminal},
};
use anyhow::{Context, Result, bail, ensure};
use russh::{ChannelMsg, Disconnect, client};
use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
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
}
#[derive(Clone, Debug)]
pub struct RemoteFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: String,
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
    let _worker = AbortOnDrop(tokio::spawn(sftp_worker(
        id.into(),
        ssh.clone(),
        events.clone(),
        job_rx,
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
                Some(job)=>if let Err(error)=job_tx.try_send(job){
                    events.send(Event::Transfer{id:id.into(),message:format!("文件任务队列繁忙：{error}"),error:true}).await.context("界面已关闭")?;
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
fn remote_join(directory: &str, name: &str) -> Result<String> {
    validate_remote(directory)?;
    ensure!(
        !name.is_empty()
            && !name.contains(['/', '\\'])
            && !name.chars().any(char::is_control)
            && name != "."
            && name != "..",
        "文件名无效"
    );
    Ok(format!("{}/{}", directory.trim_end_matches('/'), name))
}
async fn sftp_worker(
    id: String,
    ssh: Arc<client::Handle<Client>>,
    events: mpsc::Sender<Event>,
    mut jobs: mpsc::Receiver<Command>,
) {
    let mut sftp = None;
    while let Some(job) = jobs.recv().await {
        let result = async {
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
            execute_file_job(&id, client, &events, job).await
        }
        .await;
        if let Err(e) = result {
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

async fn execute_file_job(
    id: &str,
    sftp: &SftpSession,
    events: &mpsc::Sender<Event>,
    job: Command,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    match job {
        Command::List(path) => {
            validate_remote(&path)?;
            let path = sftp.canonicalize(path).await?;
            let mut entries = Vec::new();
            for entry in sftp.read_dir(path.clone()).await? {
                ensure!(entries.len() < 100000, "目录项目过多，请缩小浏览范围");
                let meta = entry.metadata();
                let name = entry.file_name();
                if name.chars().any(char::is_control) || name.contains('/') {
                    continue;
                }
                entries.push(RemoteFile {
                    name,
                    path: entry.path(),
                    size: meta.size.unwrap_or(0),
                    is_dir: meta.is_dir(),
                    modified: meta.mtime.map(|n| n.to_string()).unwrap_or_default(),
                });
            }
            entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            events
                .send(Event::Files {
                    id: id.into(),
                    path,
                    entries,
                })
                .await
                .context("界面已关闭")?;
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
            events
                .send(Event::Transfer {
                    id: id.into(),
                    message: format!("已上传 {name}，{size} 字节"),
                    error: false,
                })
                .await
                .context("界面已关闭")?;
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
            events
                .send(Event::Transfer {
                    id: id.into(),
                    message: format!("已下载 {}，{size} 字节", local.display()),
                    error: false,
                })
                .await
                .context("界面已关闭")?;
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
