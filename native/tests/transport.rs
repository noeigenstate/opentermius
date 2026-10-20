//! Real loopback SSH + SFTP protocol tests; no external server or credentials.
use opentermius::{
    ssh::{self, Command, Event},
    store::Profile,
};
use russh::{
    Channel, ChannelId,
    server::{self, Auth, Server as _},
};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

#[derive(Clone, Default)]
struct State {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    auth: Arc<AtomicUsize>,
    sizes: Arc<Mutex<Vec<(u32, u32)>>>,
    keyboard: bool,
    accepted_key: Arc<Mutex<Option<String>>>,
    disconnected: Arc<AtomicUsize>,
    terminal_queries: Arc<AtomicUsize>,
    received_responses: Arc<Mutex<Vec<u8>>>,
}
#[derive(Clone)]
struct Server(State);
impl server::Server for Server {
    type Handler = Peer;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
        Peer {
            state: self.0.clone(),
            channels: HashMap::new(),
        }
    }
}
struct Peer {
    state: State,
    channels: HashMap<ChannelId, Channel<server::Msg>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.state.disconnected.fetch_add(1, Ordering::SeqCst);
    }
}
impl server::Handler for Peer {
    type Error = anyhow::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.state.auth.fetch_add(1, Ordering::SeqCst);
        Ok(
            if !self.state.keyboard && user == "tester" && password == "test-password" {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.state.auth.fetch_add(1, Ordering::SeqCst);
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        Ok(
            if user == "tester"
                && self.state.accepted_key.lock().unwrap().as_ref() == Some(&fingerprint)
            {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if !self.state.keyboard {
            return Ok(Auth::reject());
        }
        if let Some(mut response) = response {
            return Ok(
                if response.next().as_deref() == Some(b"123456".as_slice()) {
                    Auth::Accept
                } else {
                    Auth::reject()
                },
            );
        }
        Ok(Auth::Partial {
            name: "Test OTP".into(),
            instructions: "".into(),
            prompts: vec![("Code:".into(), false)].into(),
        })
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.state.sizes.lock().unwrap().push((cols, rows));
        if self.state.terminal_queries.load(Ordering::SeqCst) > 0 {
            s.data(id, b"\x1b[6n".to_vec())?;
        }
        s.channel_success(id)?;
        Ok(())
    }
    async fn window_change_request(
        &mut self,
        _: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.state.sizes.lock().unwrap().push((cols, rows));
        Ok(())
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.channel_success(id)?;
        s.data(id, b"\x1b[32mready\x1b[0m\r\n".to_vec())?;
        if self.state.terminal_queries.load(Ordering::SeqCst) > 0 {
            s.data(id, b"\x1b[3;4H\x1b[6n".to_vec())?;
        }
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.channels.contains_key(&id) {
            if self.state.terminal_queries.load(Ordering::SeqCst) > 0 {
                if data == b"ask-query" {
                    s.data(id, b"\x1b[10;20H\x1b[6n\x1b[5n".to_vec())?;
                } else {
                    self.state
                        .received_responses
                        .lock()
                        .unwrap()
                        .extend_from_slice(data);
                }
            } else {
                s.data(id, data.to_vec())?;
            }
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            s.channel_success(id)?;
            let channel = self.channels.remove(&id).unwrap();
            let files = self.state.files.clone();
            tokio::spawn(async move {
                russh_sftp::server::run(
                    channel.into_stream(),
                    FileServer {
                        files,
                        listed: false,
                    },
                )
                .await;
            });
        } else {
            s.channel_failure(id)?;
        }
        Ok(())
    }
}
struct FileServer {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    listed: bool,
}
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}
impl russh_sftp::server::Handler for FileServer {
    type Error = StatusCode;
    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }
    async fn realpath(&mut self, id: u32, _: String) -> Result<Name, StatusCode> {
        Ok(Name {
            id,
            files: vec![File::new("/", FileAttributes::default())],
        })
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        self.listed = false;
        Ok(Handle { id, handle: path })
    }
    async fn readdir(&mut self, id: u32, _: String) -> Result<Name, StatusCode> {
        if self.listed {
            return Err(StatusCode::Eof);
        }
        self.listed = true;
        let files = self
            .files
            .lock()
            .unwrap()
            .iter()
            .map(|(name, data)| {
                File::new(
                    name.trim_start_matches('/'),
                    FileAttributes {
                        size: Some(data.len() as u64),
                        permissions: Some(0o100644),
                        ..Default::default()
                    },
                )
            })
            .collect();
        Ok(Name { id, files })
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let files = self.files.lock().unwrap();
        let data = files.get(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes {
                size: Some(data.len() as u64),
                ..Default::default()
            },
        })
    }
    async fn open(
        &mut self,
        id: u32,
        name: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if flags.contains(OpenFlags::EXCLUDE) && files.contains_key(&name) {
            return Err(StatusCode::Failure);
        }
        if flags.contains(OpenFlags::CREATE) {
            files.entry(name.clone()).or_default();
        }
        if !files.contains_key(&name) {
            return Err(StatusCode::NoSuchFile);
        }
        if flags.contains(OpenFlags::TRUNCATE) {
            files.get_mut(&name).unwrap().clear()
        }
        Ok(Handle { id, handle: name })
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }
    async fn read(
        &mut self,
        id: u32,
        name: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, StatusCode> {
        let files = self.files.lock().unwrap();
        let file = files.get(&name).ok_or(StatusCode::NoSuchFile)?;
        let start = offset as usize;
        if start >= file.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data {
            id,
            data: file[start..(start + len as usize).min(file.len())].to_vec(),
        })
    }
    async fn write(
        &mut self,
        id: u32,
        name: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        let file = files.get_mut(&name).ok_or(StatusCode::NoSuchFile)?;
        let start = offset as usize;
        file.resize(file.len().max(start + data.len()), 0);
        file[start..start + data.len()].copy_from_slice(&data);
        Ok(ok(id))
    }
}
async fn fixture(keyboard: bool) -> (u16, String, State, tokio::task::JoinHandle<()>) {
    let key = russh::keys::PrivateKey::random(
        &mut russh::keys::key::safe_rng(),
        russh::keys::Algorithm::Ed25519,
    )
    .unwrap();
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let state = State {
        keyboard,
        ..Default::default()
    };
    let mut server = Server(state.clone());
    let task = tokio::spawn(async move {
        server.run_on_socket(config, &socket).await.unwrap();
    });
    (port, fingerprint, state, task)
}
fn profile(port: u16) -> Profile {
    Profile {
        id: "test".into(),
        name: "loopback".into(),
        host: "127.0.0.1".into(),
        port,
        username: "tester".into(),
        auth_type: "password".into(),
        private_key_path: String::new(),
        key_id: String::new(),
        group: String::new(),
        encrypted_secret: None,
    }
}
async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("transport timed out")
        .expect("event queue closed")
}
async fn connected(rx: &mut mpsc::Receiver<Event>) {
    loop {
        match next(rx).await {
            Event::Status {
                connected: true, ..
            } => break,
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_host_terminal_resize_sftp_and_no_clobber() {
    let (port, key, state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(128);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        Some(key),
        tx,
    );
    connected(&mut rx).await;
    session
        .commands
        .send(Command::Resize { cols: 80, rows: 24 })
        .await
        .unwrap();
    session
        .commands
        .send(Command::Input("hello 世界\r\n".as_bytes().to_vec()))
        .await
        .unwrap();
    loop {
        if let Event::Screen { snapshot, .. } = next(&mut rx).await {
            if snapshot.lines.iter().any(|s| s.contains("hello 世界")) {
                assert_eq!((snapshot.cols, snapshot.rows), (80, 24));
                break;
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("sample.bin");
    let data = (0..524288).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    tokio::fs::write(&local, &data).await.unwrap();
    session
        .commands
        .send(Command::Upload {
            directory: "/".into(),
            local: local.clone(),
            overwrite: false,
        })
        .await
        .unwrap();
    loop {
        if let Event::Transfer { message, error, .. } = next(&mut rx).await {
            assert!(!error, "{message}");
            break;
        }
    }
    assert_eq!(state.files.lock().unwrap().get("/sample.bin"), Some(&data));
    session
        .commands
        .send(Command::Upload {
            directory: "/".into(),
            local: local.clone(),
            overwrite: false,
        })
        .await
        .unwrap();
    loop {
        if let Event::OverwriteRequired { local: actual, .. } = next(&mut rx).await {
            assert_eq!(actual, local);
            break;
        }
    }
    session
        .commands
        .send(Command::List("/".into()))
        .await
        .unwrap();
    loop {
        if let Event::Files { entries, .. } = next(&mut rx).await {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].size, data.len() as u64);
            break;
        }
    }
    let saved = dir.path().join("saved.bin");
    session
        .commands
        .send(Command::Download {
            remote: "/sample.bin".into(),
            local: saved.clone(),
        })
        .await
        .unwrap();
    loop {
        if let Event::Transfer { message, error, .. } = next(&mut rx).await {
            assert!(!error, "{message}");
            break;
        }
    }
    assert_eq!(tokio::fs::read(&saved).await.unwrap(), data);
    session
        .commands
        .send(Command::Download {
            remote: "/sample.bin".into(),
            local: saved.clone(),
        })
        .await
        .unwrap();
    loop {
        if let Event::Transfer { error, .. } = next(&mut rx).await {
            assert!(error);
            break;
        }
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no abandoned partial download"
    );
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(5), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_key_rejection_and_changed_key_never_authenticate() {
    let (port, _key, state, server) = fixture(false).await;
    for known in [None, Some("SHA256:untrusted".into())] {
        let (tx, mut rx) = mpsc::channel(16);
        let session = ssh::spawn(
            &tokio::runtime::Handle::current(),
            "test".into(),
            profile(port),
            "test-password".into(),
            known,
            tx,
        );
        let mut failed = false;
        loop {
            match next(&mut rx).await {
                Event::HostKey { reply, .. } => {
                    reply.send(false).unwrap();
                }
                Event::Status { error: true, .. } => failed = true,
                Event::Closed { .. } => break,
                Event::Status {
                    connected: true, ..
                } => panic!("untrusted connection established"),
                _ => {}
            }
        }
        assert!(failed);
        assert_eq!(state.auth.load(Ordering::SeqCst), 0);
        session.task.await.unwrap();
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_key_approval_then_keyboard_interactive() {
    let (port, key, _state, server) = fixture(true).await;
    let (tx, mut rx) = mpsc::channel(16);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "wrong".into(),
        None,
        tx,
    );
    let (mut trusted, mut prompted) = (false, false);
    loop {
        match next(&mut rx).await {
            Event::HostKey {
                fingerprint, reply, ..
            } => {
                assert_eq!(fingerprint, key);
                trusted = true;
                reply.send(true).unwrap();
            }
            Event::AuthPrompt { prompts, reply, .. } => {
                assert!(trusted);
                assert_eq!(prompts, vec![("Code:".into(), false)]);
                prompted = true;
                reply.send(Some(vec!["123456".into()])).unwrap();
            }
            Event::Status {
                connected: true, ..
            } => break,
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            _ => {}
        }
    }
    assert!(prompted);
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(5), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_cancels_pending_trust_without_waiting_for_prompt_timeout() {
    let (port, _key, state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(16);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        None,
        tx,
    );
    let _pending = loop {
        if let Event::HostKey { reply, .. } = next(&mut rx).await {
            break reply;
        }
    };
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(2), session.task)
        .await
        .expect("disconnect waited for host trust")
        .unwrap();
    assert_eq!(state.auth.load(Ordering::SeqCst), 0);
    timeout(Duration::from_secs(2), async {
        while state.disconnected.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled host trust leaked the SSH socket");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn congested_screen_notifications_preserve_all_network_bytes() {
    let (port, key, _state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(2);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        Some(key),
        tx,
    );
    connected(&mut rx).await;
    let mut payload = b"\x1b[2J\x1b[H".to_vec();
    for i in 0..20000 {
        payload.extend_from_slice(format!("line {i}\r\n").as_bytes())
    }
    payload.extend_from_slice(b"ALL_BYTES_PARSED");
    session
        .commands
        .send(Command::Input(payload))
        .await
        .unwrap();
    // A completely full frame queue must not stall parsing or corrupt VT state.
    tokio::time::sleep(Duration::from_millis(250)).await;
    loop {
        match next(&mut rx).await {
            Event::Screen { snapshot, .. }
                if snapshot
                    .lines
                    .iter()
                    .any(|s| s.contains("ALL_BYTES_PARSED")) =>
            {
                break;
            }
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            _ => {}
        }
    }
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(5), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_private_key_authenticates_with_supplied_passphrase() {
    let (port, known, state, server) = fixture(false).await;
    let key = russh::keys::PrivateKey::random(
        &mut russh::keys::key::safe_rng(),
        russh::keys::Algorithm::Ed25519,
    )
    .unwrap();
    *state.accepted_key.lock().unwrap() = Some(
        key.public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string(),
    );
    let encoded = key
        .encrypt(&mut russh::keys::key::safe_rng(), "test-passphrase")
        .unwrap()
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-key");
    tokio::fs::write(&path, encoded.as_bytes()).await.unwrap();
    let mut p = profile(port);
    p.auth_type = "key".into();
    p.private_key_path = path.to_str().unwrap().into();
    let (tx, mut rx) = mpsc::channel(16);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        p,
        "test-passphrase".into(),
        Some(known),
        tx,
    );
    connected(&mut rx).await;
    assert_eq!(state.auth.load(Ordering::SeqCst), 1);
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(5), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_queries_during_setup_and_normal_io_reach_server() {
    let (port, known, state, server) = fixture(false).await;
    state.terminal_queries.store(1, Ordering::SeqCst);
    let (tx, mut rx) = mpsc::channel(16);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        Some(known),
        tx,
    );
    connected(&mut rx).await;
    session
        .commands
        .send(Command::Input(b"ask-query".to_vec()))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            let complete = {
                let data = state.received_responses.lock().unwrap();
                [
                    b"\x1b[1;1R".as_slice(),
                    b"\x1b[3;4R",
                    b"\x1b[10;20R",
                    b"\x1b[0n",
                ]
                .iter()
                .all(|reply| data.windows(reply.len()).any(|chunk| chunk == *reply))
            };
            if complete {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal DSR replies did not reach SSH server");
    session.commands.send(Command::Disconnect).await.unwrap();
    timeout(Duration::from_secs(5), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

// This deliberately exercises the real native window and requires an interactive
// Windows desktop. Keep it out of headless/default unit-test runs.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an interactive Windows desktop; launches the native app"]
async fn native_gui_live_selftest() {
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct ServerGuard(tokio::task::JoinHandle<()>);
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let (port, _key, state, server) = fixture(false).await;
    let _server = ServerGuard(server);
    state
        .files
        .lock()
        .unwrap()
        .insert("/smoke.txt".into(), b"native smoke file".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let output = std::env::var_os("OPENTERMIUS_SMOKE_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.path().join("output"));
    std::fs::create_dir_all(&output).unwrap();
    let executable = std::env::var_os("OPENTERMIUS_TEST_EXECUTABLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_opentermius").into());
    let mut child = ChildGuard(
        std::process::Command::new(executable)
            .arg("--smoke-test")
            .arg(format!("--smoke-port={port}"))
            .arg(format!("--data-dir={}", dir.path().join("data").display()))
            .arg(format!("--output={}", output.display()))
            .spawn()
            .expect("failed to launch native app"),
    );
    let child_pid = child.0.id();
    let status = timeout(Duration::from_secs(25), async {
        loop {
            if let Some(status) = child.0.try_wait().expect("cannot poll native app") {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("native GUI self-test timed out; spawned app will be terminated");
    let error = std::fs::read_to_string(output.join("error.txt")).unwrap_or_default();
    assert!(status.success(), "native app failed: {error}");
    let marker = std::fs::read(output.join("startup.json"))
        .unwrap_or_else(|e| panic!("missing native self-test marker: {e}; {error}"));
    let marker: serde_json::Value =
        serde_json::from_slice(&marker).expect("invalid self-test marker");
    assert_eq!(
        marker["ok"], true,
        "native self-test failed: {marker}; {error}"
    );
    assert_eq!(
        marker["pid"].as_u64(),
        Some(u64::from(child_pid)),
        "self-test marker belongs to an earlier process"
    );
    assert!(
        marker["checks"]
            .as_array()
            .is_some_and(|checks| checks.len() >= 10),
        "native app did not complete the live protocol/UI checks: {marker}"
    );
    let screenshot = output.join("native-window.png");
    let image = image::open(&screenshot).expect("native screenshot missing or invalid");
    assert!(
        image.width() >= 800 && image.height() >= 500,
        "unexpected native screenshot dimensions"
    );
    assert!(
        state.auth.load(Ordering::SeqCst) > 0,
        "GUI self-test never authenticated against real SSH fixture"
    );
}
