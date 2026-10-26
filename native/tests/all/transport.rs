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
    stats_calls: Arc<AtomicUsize>,
    os_calls: Arc<Mutex<Vec<String>>>,
    os_outputs: Arc<Mutex<HashMap<String, String>>>,
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
    // Answers the usage poller like a small Linux server whose CPU counters advance.
    async fn exec_request(
        &mut self,
        id: ChannelId,
        command: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if [opentermius::host_os::COMMAND.as_bytes(), opentermius::host_os::WINDOWS_COMMAND.as_bytes()].contains(&command) {
            let command = String::from_utf8_lossy(command).into_owned();
            self.state.os_calls.lock().unwrap().push(command.clone());
            let output = self.state.os_outputs.lock().unwrap().get(&command).cloned();
            if let Some(output) = output {
                s.channel_success(id)?;
                s.data(id, output.into_bytes())?;
            } else {
                s.channel_failure(id)?;
            }
            s.eof(id)?;
            s.close(id)?;
            return Ok(());
        }
        if command != opentermius::server_stats::COMMAND.as_bytes() {
            s.channel_failure(id)?;
            return Ok(());
        }
        let call = self.state.stats_calls.fetch_add(1, Ordering::SeqCst) as u64;
        s.channel_success(id)?;
        let output = format!(
            "cpu  {} 0 {} {} 0 0 0 0 0 0\nMemTotal: 4000000 kB\nMemAvailable: 1000000 kB\n/dev/sda1 100 42 58 42% /\n",
            100 + call * 30,
            100 + call * 30,
            800 + call * 40
        );
        s.data(id, output.into_bytes())?;
        s.exit_status_request(id, 0)?;
        s.eof(id)?;
        s.close(id)?;
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
        tags: Vec::new(),
        os: None,
        last_connected: None,
        favorite: false,
        created_at: None,
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
async fn os_discovery_uses_a_separate_channel_and_windows_fallback_once() {
    for (command, output, expected) in [
        (opentermius::host_os::COMMAND, "ID=ubuntu\nID_LIKE=debian\nLinux\n", "ubuntu"),
        (opentermius::host_os::WINDOWS_COMMAND, "Microsoft Windows [Version 10.0.20348.0]\r\n", "windows"),
    ] {
        let (port, key, state, server) = fixture(false).await;
        state.os_outputs.lock().unwrap().insert(command.into(), output.into());
        let (tx, mut rx) = mpsc::channel(128);
        let session = ssh::spawn(&tokio::runtime::Handle::current(), "test".into(), profile(port), "test-password".into(), Some(key), tx);
        connected(&mut rx).await;
        let detected = timeout(Duration::from_secs(8), async {
            loop {
                if let Event::OsDetected { os, .. } = next(&mut rx).await { break os; }
            }
        }).await.unwrap();
        assert_eq!(detected, expected);
        session.commands.send(Command::Input(b"probe-completed".to_vec())).await.unwrap();
        loop {
            match next(&mut rx).await {
                Event::Output { data, .. } if data.windows(15).any(|part| part == b"probe-completed") => break,
                Event::OsDetected { .. } => panic!("OS was probed more than once"),
                _ => {}
            }
        }
        let expected_calls = if expected == "windows" {
            vec![opentermius::host_os::COMMAND, opentermius::host_os::WINDOWS_COMMAND]
        } else {
            vec![opentermius::host_os::COMMAND]
        };
        assert_eq!(*state.os_calls.lock().unwrap(), expected_calls);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_os_discovery_leaves_the_terminal_and_stats_working() {
    let (port, key, state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(128);
    let session = ssh::spawn(&tokio::runtime::Handle::current(), "test".into(), profile(port), "test-password".into(), Some(key), tx);
    connected(&mut rx).await;
    // Both probe commands are rejected; stats still arrive on their own channel.
    loop {
        match next(&mut rx).await {
            Event::Stats { .. } => break,
            Event::OsDetected { .. } => panic!("failed discovery produced an OS"),
            _ => {}
        }
    }
    assert_eq!(state.os_calls.lock().unwrap().len(), 2);
    session.commands.send(Command::Input(b"still-ready".to_vec())).await.unwrap();
    loop {
        match next(&mut rx).await {
            Event::Output { data, .. } if data.windows(11).any(|part| part == b"still-ready") => break,
            Event::OsDetected { .. } => panic!("failed discovery produced an OS"),
            _ => {}
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_sessions_report_server_usage() {
    let (port, key, state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(128);
    let _session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        Some(key),
        tx,
    );
    connected(&mut rx).await;
    // The first reading has memory and disk; CPU needs a second sample.
    let stats = timeout(Duration::from_secs(10), async {
        loop {
            if let Event::Stats { stats, .. } = next(&mut rx).await {
                if stats.cpu.is_some() {
                    break stats;
                }
                assert_eq!((stats.memory, stats.disk), (Some(75), Some(42)));
            }
        }
    })
    .await
    .expect("no usage reading with CPU");
    // Each poll adds 60 busy of 100 total ticks.
    assert_eq!(
        (stats.cpu, stats.memory, stats.disk),
        (Some(60), Some(75), Some(42))
    );
    assert!(state.stats_calls.load(Ordering::SeqCst) >= 2);
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_pty_request_uses_the_window_size() {
    let (port, key, state, server) = fixture(false).await;
    let (tx, mut rx) = mpsc::channel(128);
    let _session = ssh::spawn_with_options(
        &tokio::runtime::Handle::current(),
        "test".into(),
        profile(port),
        "test-password".into(),
        Some(key),
        tx,
        ssh::ConnectOptions {
            initial_size: (132, 40),
            ..Default::default()
        },
    );
    connected(&mut rx).await;
    // The banner must be laid out at the real size, not at a default fixed later by a resize.
    assert_eq!(
        state.sizes.lock().unwrap().first().copied(),
        Some((132, 40))
    );
    server.abort();
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
    let mut output = Vec::new();
    loop {
        if let Event::Output { data, .. } = next(&mut rx).await {
            assert!(data.len() <= 65536);
            output.extend(data);
            if String::from_utf8_lossy(&output).contains("hello \u{4e16}\u{754c}") {
                break;
            }
        }
    }
    assert!(state.sizes.lock().unwrap().contains(&(80, 24)));
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("sample.bin");
    let data = (0..524288).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    tokio::fs::write(&local, &data).await.unwrap();
    session
        .commands
        .send(Command::Upload {
            local_root: dir.path().to_path_buf(),
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
            local_root: dir.path().to_path_buf(),
            directory: "/".into(),
            local: local.clone(),
            overwrite: false,
        })
        .await
        .unwrap();
    loop {
        if let Event::OverwriteRequired { local: actual, .. } = next(&mut rx).await {
            assert_eq!(actual, local.canonicalize().unwrap());
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
            local_root: dir.path().to_path_buf(),
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
            local_root: dir.path().to_path_buf(),
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
    let (port, key, state, server) = fixture(true).await;
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
                session
                    .commands
                    .send(Command::Resize {
                        cols: 144,
                        rows: 45,
                    })
                    .await
                    .unwrap();
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
    assert_eq!(
        state.sizes.lock().unwrap().first().copied(),
        Some((144, 45))
    );
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
async fn congested_output_notifications_preserve_all_network_bytes() {
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
        .send(Command::Input(payload.clone()))
        .await
        .unwrap();
    // A full output queue preserves every byte in order when consumption resumes.
    tokio::time::sleep(Duration::from_millis(250)).await;
    let mut output = Vec::new();
    loop {
        match next(&mut rx).await {
            Event::Output { data, .. } => {
                assert!(!data.is_empty() && data.len() <= 65536);
                output.extend(data);
                if output.ends_with(b"ALL_BYTES_PARSED") {
                    break;
                }
            }
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            _ => {}
        }
    }
    assert!(output.ends_with(&payload));
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
async fn terminal_queries_stream_unchanged_and_frontend_responses_reach_server() {
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
    let mut output = Vec::new();
    timeout(Duration::from_secs(5), async {
        while !output.windows(4).any(|bytes| bytes == b"\x1b[5n") {
            if let Event::Output { data, .. } = next(&mut rx).await {
                output.extend(data);
            }
        }
    })
    .await
    .unwrap();
    assert!(output.windows(4).any(|bytes| bytes == b"\x1b[6n"));
    assert!(state.received_responses.lock().unwrap().is_empty());
    // The frontend answers terminal queries through ordinary input.
    session
        .commands
        .send(Command::Input(
            b"\x1b[1;1R\x1b[3;4R\x1b[10;20R\x1b[0n".to_vec(),
        ))
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
