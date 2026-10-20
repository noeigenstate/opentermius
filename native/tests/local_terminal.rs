#![cfg(windows)]
//! Real ConPTY + PowerShell tests. No profile files or persisted settings.
use opentermius::{
    local,
    ssh::{Command, Event, Session},
    terminal::Snapshot,
};
use std::time::Duration;
use tokio::{sync::mpsc, time::timeout};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}
fn alive(pid: u32) -> bool {
    // SYNCHRONIZE permits waiting without process-memory access.
    unsafe {
        let handle = OpenProcess(0x00100000, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == 258;
        CloseHandle(handle);
        running
    }
}
async fn stopped(pid: u32) {
    timeout(Duration::from_secs(5), async {
        while alive(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("local shell process survived session cleanup");
}
async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("local terminal response timed out")
        .expect("local event queue closed")
}
async fn screen(rx: &mut mpsc::Receiver<Event>, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    timeout(Duration::from_secs(20), async {
        loop {
            match next(rx).await {
                Event::Screen { snapshot, .. } if predicate(&snapshot) => return snapshot,
                Event::Status {
                    error: true,
                    message,
                    ..
                } => panic!("local shell failed: {message}"),
                Event::Closed { .. } => {
                    panic!("local shell closed before producing expected output")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("expected local shell output never arrived")
}
async fn start() -> (Session, mpsc::Receiver<Event>, u32) {
    let (tx, mut rx) = mpsc::channel(16);
    let session = local::spawn(&tokio::runtime::Handle::current(), "local-test".into(), tx);
    loop {
        match next(&mut rx).await {
            Event::Status {
                connected: true, ..
            } => break,
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            Event::Closed { .. } => panic!("local shell startup failed"),
            _ => {}
        }
    }
    session
        .commands
        .send(Command::Input(
            b"Write-Output ('LOCAL_PID:'+$PID)\r".to_vec(),
        ))
        .await
        .unwrap();
    let snapshot = screen(&mut rx, |s| {
        s.lines.iter().any(|line| {
            line.trim()
                .strip_prefix("LOCAL_PID:")
                .and_then(|pid| pid.parse::<u32>().ok())
                .is_some()
        })
    })
    .await;
    let pid = snapshot
        .lines
        .iter()
        .find_map(|line| {
            line.trim()
                .strip_prefix("LOCAL_PID:")
                .and_then(|pid| pid.parse().ok())
        })
        .unwrap();
    assert!(alive(pid), "local shell process was not alive");
    (session, rx, pid)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_powershell_unicode_resize_sftp_rejection_and_disconnect() {
    let (session, mut rx, pid) = start().await;
    session
        .commands
        .send(Command::Input(
            b"Write-Output ('UNICODE:'+[char]0x4f60+[char]0x597d)\r".to_vec(),
        ))
        .await
        .unwrap();
    screen(&mut rx, |s| {
        s.lines.iter().any(|line| line.contains("UNICODE:你好"))
    })
    .await;
    session
        .commands
        .send(Command::Resize { cols: 80, rows: 24 })
        .await
        .unwrap();
    screen(&mut rx, |s| s.cols == 80 && s.rows == 24).await;
    session.commands.send(Command::Input(b"Write-Output ('LOCAL_SIZE:'+$Host.UI.RawUI.WindowSize.Width+','+$Host.UI.RawUI.WindowSize.Height)\r".to_vec())).await.unwrap();
    screen(&mut rx, |s| {
        s.lines.iter().any(|line| line.trim() == "LOCAL_SIZE:80,24")
    })
    .await;
    session
        .commands
        .send(Command::List(".".into()))
        .await
        .unwrap();
    loop {
        if let Event::Transfer { message, error, .. } = next(&mut rx).await {
            assert!(error);
            assert!(message.contains("SFTP"));
            break;
        }
    }
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(8), session.task)
        .await
        .expect("local disconnect timed out")
        .unwrap();
    stopped(pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_session_terminates_owned_local_shell() {
    let (session, _rx, pid) = start().await;
    session.task.abort();
    let _ = session.task.await;
    stopped(pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_shell_exit_closes_session_and_process() {
    let (session, mut rx, pid) = start().await;
    session
        .commands
        .send(Command::Input(b"exit\r".to_vec()))
        .await
        .unwrap();
    loop {
        if matches!(next(&mut rx).await, Event::Closed { .. }) {
            break;
        }
    }
    timeout(Duration::from_secs(8), session.task)
        .await
        .expect("local EOF cleanup timed out")
        .unwrap();
    stopped(pid).await;
}
