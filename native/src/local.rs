//! Native local shell backed by portable-pty (ConPTY on Windows). Blocking I/O never runs on
//! the UI thread; the reader parses all bytes independently of screen delivery.
use crate::{
    ssh::{Command, Event, Session},
    terminal::Terminal,
};
use anyhow::{Context, Result, bail, ensure};
use portable_pty::{CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
    time::{MissedTickBehavior, interval},
};

pub fn spawn(rt: &Handle, id: String, events: mpsc::Sender<Event>) -> Session {
    let (commands, rx) = mpsc::channel(64);
    let session_id = id.clone();
    let task = rt.spawn(async move {
        if let Err(error) = run(&id, &events, rx).await {
            let _ = events
                .send(Event::Status {
                    id: id.clone(),
                    connected: false,
                    message: format!("{error:#}"),
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
struct Screen {
    terminal: Terminal,
    dirty: bool,
    error: Option<String>,
}
impl Screen {
    fn fail(&mut self, error: impl ToString) {
        if self.error.is_none() {
            self.error = Some(error.to_string());
        }
    }
}
enum Control {
    Resize { cols: u16, rows: u16 },
    Disconnect,
}
struct Owner {
    control: mpsc::Sender<Control>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.control.try_send(Control::Disconnect);
    }
}
// Keep the child guard before the master so termination precedes ConPTY close.
struct ChildGuard(Box<dyn portable_pty::Child + Send + Sync>);
impl ChildGuard {
    fn terminate(&mut self) {
        // portable-pty sends SIGHUP by PID on Unix, even after wait() reaped it.
        // Do not signal a cached exit status: the PID may already be reused.
        #[cfg(unix)]
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(unix)]
fn select_shell(preferred: Option<std::ffi::OsString>) -> Result<std::path::PathBuf> {
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};
    let usable = |path: &PathBuf| {
        path.is_absolute()
            && path
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if let Some(path) = preferred.map(PathBuf::from).filter(usable) {
        return Ok(path);
    }
    #[cfg(target_os = "macos")]
    let fallbacks = ["/bin/zsh", "/bin/sh"];
    #[cfg(not(target_os = "macos"))]
    let fallbacks = ["/bin/bash", "/bin/sh"];
    fallbacks
        .into_iter()
        .map(PathBuf::from)
        .find(usable)
        .context(crate::i18n::t("No local shell is available"))
}

#[cfg(all(test, unix))]
mod shell_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn shell_selection_honors_executables_and_falls_back_for_invalid_shell() {
        let temp = tempfile::tempdir().unwrap();
        let shell = temp.path().join("custom-shell");
        std::fs::write(&shell, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(select_shell(Some(shell.clone().into())).unwrap(), shell);
        let default = select_shell(None).unwrap();
        for invalid in ["", "relative-shell", "/missing-opentermius-shell"] {
            assert_eq!(select_shell(Some(invalid.into())).unwrap(), default);
        }
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(select_shell(Some(shell.into())).unwrap(), default);
    }
}

fn supervise(
    screen: Arc<Mutex<Screen>>,
    eof: Arc<AtomicBool>,
    mut controls: mpsc::Receiver<Control>,
    writes: mpsc::Sender<Vec<u8>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
    ready: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let system = portable_pty::native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context(if cfg!(windows) {
            crate::i18n::t("Cannot create a local pseudo-terminal; check that Windows supports ConPTY")
        } else {
            crate::i18n::t("Cannot create a local pseudo-terminal")
        })?;
    #[cfg(windows)]
    let executable = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    #[cfg(unix)]
    let executable = select_shell(std::env::var_os("SHELL"))?;
    let mut command = CommandBuilder::new(executable);
    #[cfg(windows)]
    {
        command.arg("-NoLogo");
        command.arg("-NoProfile");
    }
    #[cfg(unix)]
    command.arg("-l");
    command.env("TERM", "xterm-256color");
    let mut child = ChildGuard(
        pair.slave
            .spawn_command(command)
            .context(if cfg!(windows) {
                crate::i18n::t("Cannot start local PowerShell")
            } else {
                crate::i18n::t("Cannot start local shell")
            })?,
    );
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    drop(pair.slave);
    let master = pair.master;
    let reader_screen = screen.clone();
    let reader_eof = eof.clone();
    let reading = std::thread::Builder::new()
        .name("opentermius-pty-read".into())
        .spawn(move || {
            let mut bytes = [0u8; 8192];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        let Ok(mut state) = reader_screen.lock() else {
                            break;
                        };
                        state.terminal.feed(&bytes[..count]);
                        state.dirty = true;
                        let replies = state.terminal.take_responses();
                        if !replies.is_empty() && writes.try_send(replies).is_err() {
                            state.fail(crate::i18n::t("Local terminal input queue is full; cannot answer terminal query"));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Windows reports a closed ConPTY output pipe as BrokenPipe.
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
                        ) =>
                    {
                        break;
                    }
                    Err(error) => {
                        if let Ok(mut state) = reader_screen.lock() {
                            state.fail(error);
                        }
                        break;
                    }
                }
            }
            reader_eof.store(true, Ordering::Release);
        })
        .context(crate::i18n::t("Cannot start terminal reader thread"))?;
    let writer_screen = screen.clone();
    let writing = std::thread::Builder::new()
        .name("opentermius-pty-write".into())
        .spawn(move || {
            while let Some(data) = write_rx.blocking_recv() {
                if let Err(error) = writer.write_all(&data).and_then(|_| writer.flush()) {
                    if let Ok(mut state) = writer_screen.lock() {
                        state.fail(error);
                    }
                    break;
                }
            }
        })
        .context(crate::i18n::t("Cannot start terminal input thread"))?;
    let accepted = ready.send(Ok(())).is_ok();
    if accepted {
        loop {
            match controls.try_recv() {
                Ok(control) => match control {
                    Control::Disconnect => break,
                    Control::Resize { cols, rows } => {
                        if let Err(error) = master.resize(PtySize {
                            cols,
                            rows,
                            pixel_width: 0,
                            pixel_height: 0,
                        }) {
                            if let Ok(mut state) = screen.lock() {
                                state.fail(error);
                            }
                        }
                    }
                },
                Err(mpsc::error::TryRecvError::Disconnected) => break,
                Err(mpsc::error::TryRecvError::Empty) => {
                    if child.0.try_wait()?.is_some() {
                        break;
                    }
                    std::thread::park_timeout(Duration::from_millis(20));
                }
            }
        }
    }
    // Closing the pseudoconsole can wait for its output pipe. Keep the reader
    // alive and draining until close completes, even when the owning task aborts.
    child.terminate();
    drop(child);
    drop(master);
    let _ = reading.join();
    let _ = writing.join();
    Ok(())
}

async fn run(
    id: &str,
    events: &mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
) -> Result<()> {
    let screen = Arc::new(Mutex::new(Screen {
        terminal: Terminal::new(30, 100),
        dirty: true,
        error: None,
    }));
    let eof = Arc::new(AtomicBool::new(false));
    let (control, controls) = mpsc::channel(8);
    let owner = Owner { control };
    let (writes, write_rx) = mpsc::channel(64);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (worker_screen, worker_eof) = (screen.clone(), eof.clone());
    let worker_writes = writes.clone();
    let (done_tx, worker) = oneshot::channel();
    std::thread::Builder::new()
        .name("opentermius-pty-owner".into())
        .spawn(move || {
            let result = supervise(
                worker_screen,
                worker_eof,
                controls,
                worker_writes,
                write_rx,
                ready_tx,
            );
            let _ = done_tx.send(result);
        })
        .context(crate::i18n::t("Cannot start local terminal manager thread"))?;
    tokio::pin!(ready_rx);
    loop {
        tokio::select! {
            result=&mut ready_rx=>match result{
                Ok(result)=>{result?;break},
                Err(_)=>{drop(writes);drop(owner);worker.await.context(crate::i18n::t("Local terminal manager thread has exited"))??;bail!("{}", crate::i18n::t("Local terminal failed to start"))}
            },
            command=commands.recv()=>if matches!(command,None|Some(Command::Disconnect)){drop(writes);drop(owner);let _=worker.await;return Ok(())},
        }
    }
    events
        .send(Event::Status {
            id: id.into(),
            connected: true,
            message: if cfg!(windows) {
                crate::i18n::t("Local PowerShell connected")
            } else {
                crate::i18n::t("Local shell connected")
            }
            .into(),
            error: false,
        })
        .await
        .context(crate::i18n::t("Window closed"))?;
    let mut frames = interval(Duration::from_millis(34));
    frames.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending = None;
    loop {
        tokio::select! {
            permit=writes.reserve(),if pending.is_some()=>{permit.context(crate::i18n::t("Local terminal input closed"))?.send(pending.take().context(crate::i18n::t("Terminal input is missing"))?);},
            command=commands.recv(),if pending.is_none()=>match command{
                None|Some(Command::Disconnect)=>break,
                Some(Command::Input(data))=>{pending=Some(data)},
                Some(Command::Resize{cols,rows})=>{
                    let(cols,rows)=(cols.clamp(2,500),rows.clamp(2,300));
                    owner.control.send(Control::Resize{cols,rows}).await.context(crate::i18n::t("Local terminal closed"))?;
                    let mut state=screen.lock().map_err(|_|anyhow::anyhow!("{}", crate::i18n::t("Terminal state is unavailable")))?;state.terminal.resize(rows,cols);state.dirty=true;
                }
                Some(Command::Scroll(delta))=>{let mut state=screen.lock().map_err(|_|anyhow::anyhow!("{}", crate::i18n::t("Terminal state is unavailable")))?;state.terminal.scroll(delta);state.dirty=true;},
                Some(_)=>{events.send(Event::Transfer{id:id.into(),message:crate::i18n::t("Local terminals do not support SFTP. Connect to an SSH host to manage files").into(),error:true}).await.context(crate::i18n::t("Window closed"))?;}
            },
            _=frames.tick()=>{
                let mut state=screen.lock().map_err(|_|anyhow::anyhow!("{}", crate::i18n::t("Terminal state is unavailable")))?;
                ensure!(state.error.is_none(),"{}",state.error.as_deref().unwrap_or_default());
                if state.dirty{
                    let snapshot=state.terminal.snapshot();
                    match events.try_send(Event::Screen{id:id.into(),snapshot}){
                        Ok(())=>state.dirty=false,
                        Err(mpsc::error::TrySendError::Closed(_))=>break,
                        Err(mpsc::error::TrySendError::Full(_))=>{}
                    }
                }
                if eof.load(Ordering::Acquire){break}
            }
        }
    }
    drop(writes);
    drop(owner);
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .context(crate::i18n::t("Local terminal shutdown timed out"))??
        .context(crate::i18n::t("Local terminal exit failed"))?;
    Ok(())
}
