//! Native local shell backed by portable-pty (ConPTY on Windows). Blocking I/O never runs on
//! the UI thread; a bounded byte queue applies backpressure without dropping output.
use crate::ssh::{Command, Event, Session};
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
    spawn_with_size(rt, id, events, (100, 30))
}

pub fn spawn_with_size(
    rt: &Handle,
    id: String,
    events: mpsc::Sender<Event>,
    initial_size: (u16, u16),
) -> Session {
    let (commands, rx) = mpsc::channel(64);
    let session_id = id.clone();
    let task = rt.spawn(async move {
        if let Err(error) = run(&id, &events, rx, initial_size).await {
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
struct IoState {
    error: Option<String>,
}
impl IoState {
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
    io_state: Arc<Mutex<IoState>>,
    eof: Arc<AtomicBool>,
    mut controls: mpsc::Receiver<Control>,
    output: mpsc::Sender<Vec<u8>>,
    initial_size: (u16, u16),
    mut write_rx: mpsc::Receiver<Vec<u8>>,
    ready: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let system = portable_pty::native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: initial_size.1.clamp(2, 300),
            cols: initial_size.0.clamp(2, 500),
            pixel_width: 0,
            pixel_height: 0,
        })
        .context(if cfg!(windows) {
            crate::i18n::t(
                "Cannot create a local pseudo-terminal; check that Windows supports ConPTY",
            )
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
    {
        command.arg("-l");
        command.arg("-i");
    }
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
    let reader_state = io_state.clone();
    let reader_eof = eof.clone();
    let reading = std::thread::Builder::new()
        .name("opentermius-pty-read".into())
        .spawn(move || {
            let mut bytes = [0u8; 8192];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        // Abort closes the receiver and unblocks this send. Continue
                        // draining until EOF: closing ConPTY can wait on its output pipe.
                        if !output.is_closed() {
                            let _ = output.blocking_send(bytes[..count].to_vec());
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
                        if let Ok(mut state) = reader_state.lock() {
                            state.fail(error);
                        }
                        break;
                    }
                }
            }
            reader_eof.store(true, Ordering::Release);
        })
        .context(crate::i18n::t("Cannot start terminal reader thread"))?;
    let writer_state = io_state.clone();
    let writing = std::thread::Builder::new()
        .name("opentermius-pty-write".into())
        .spawn(move || {
            while let Some(data) = write_rx.blocking_recv() {
                if let Err(error) = writer.write_all(&data).and_then(|_| writer.flush()) {
                    if let Ok(mut state) = writer_state.lock() {
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
                        match master.resize(PtySize {
                            cols,
                            rows,
                            pixel_width: 0,
                            pixel_height: 0,
                        }) {
                            Ok(()) => {}
                            Err(error) => {
                                if let Ok(mut state) = io_state.lock() {
                                    state.fail(error);
                                }
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
    initial_size: (u16, u16),
) -> Result<()> {
    let io_state = Arc::new(Mutex::new(IoState { error: None }));
    let eof = Arc::new(AtomicBool::new(false));
    let (control, controls) = mpsc::channel(8);
    let owner = Owner { control };
    let (writes, write_rx) = mpsc::channel(64);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (worker_state, worker_eof) = (io_state.clone(), eof.clone());
    let (output_tx, mut output_rx) = mpsc::channel(8);
    let (done_tx, worker) = oneshot::channel();
    std::thread::Builder::new()
        .name("opentermius-pty-owner".into())
        .spawn(move || {
            let result = supervise(
                worker_state,
                worker_eof,
                controls,
                output_tx,
                initial_size,
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
            command=commands.recv()=>if matches!(command,None|Some(Command::Disconnect)){drop(output_rx);drop(writes);drop(owner);let _=worker.await;return Ok(())},
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
    let mut frames = interval(Duration::from_millis(10));
    frames.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending = None;
    let mut output = Vec::new();
    let mut flush_output = false;
    loop {
        tokio::select! {
            permit=writes.reserve(),if pending.is_some()=>{permit.context(crate::i18n::t("Local terminal input closed"))?.send(pending.take().context(crate::i18n::t("Terminal input is missing"))?);},
            command=commands.recv(),if pending.is_none()=>match command{
                None|Some(Command::Disconnect)=>{output.clear();break},
                Some(Command::Input(data))=>{pending=Some(data)},
                Some(Command::Resize{cols,rows})=>{
                    let(cols,rows)=(cols.clamp(2,500),rows.clamp(2,300));
                    owner.control.send(Control::Resize{cols,rows}).await.context(crate::i18n::t("Local terminal closed"))?;
                }
                Some(_)=>{events.send(Event::Transfer{id:id.into(),message:crate::i18n::t("Local terminals do not support SFTP. Connect to an SSH host to manage files").into(),error:true}).await.context(crate::i18n::t("Window closed"))?;}
            },
            data=output_rx.recv(),if output.len()<65536=>match data {
                Some(data)=>output.extend(data),
                None=>break,
            },
            permit=events.reserve(),if !output.is_empty()&&(flush_output||output.len()>=65536)=>{
                let data=output.drain(..output.len().min(65536)).collect();
                permit.context(crate::i18n::t("Window closed"))?.send(Event::Output{id:id.into(),data});
                flush_output=false;
            },
            _=frames.tick()=>{
                let state=io_state.lock().map_err(|_|anyhow::anyhow!("{}", crate::i18n::t("Terminal state is unavailable")))?;
                ensure!(state.error.is_none(),"{}",state.error.as_deref().unwrap_or_default());
                flush_output=true;
                if eof.load(Ordering::Acquire)&&output_rx.is_empty(){break}
            }
        }
    }
    for data in output.chunks(65536) {
        events
            .send(Event::Output {
                id: id.into(),
                data: data.to_vec(),
            })
            .await
            .context(crate::i18n::t("Window closed"))?;
    }
    drop(output_rx);
    drop(writes);
    drop(owner);
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .context(crate::i18n::t("Local terminal shutdown timed out"))??
        .context(crate::i18n::t("Local terminal exit failed"))?;
    Ok(())
}
