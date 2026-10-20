//! Real SSH forwarding against an in-process server; no external network services.
use opentermius::{
    forwarding::{Forward, Kind, Phase, RunningRule, socks5_connect},
    store::Profile,
};
use russh::{
    Channel,
    server::{self, Auth, Server as _},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use zeroize::Zeroizing;

struct Count(Arc<AtomicUsize>);
impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Clone, Default)]
struct Server {
    peers: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    accepted_key: Arc<Mutex<Option<String>>>,
}
impl server::Server for Server {
    type Handler = Peer;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
        self.peers.fetch_add(1, Ordering::SeqCst);
        Peer {
            _count: Count(self.peers.clone()),
            cancelled: self.cancelled.clone(),
            accepted_key: self.accepted_key.clone(),
            listener: None,
            jobs: JoinSet::new(),
        }
    }
}
struct Peer {
    _count: Count,
    cancelled: Arc<AtomicUsize>,
    accepted_key: Arc<Mutex<Option<String>>>,
    listener: Option<JoinHandle<()>>,
    jobs: JoinSet<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(task) = self.listener.take() {
            task.abort();
        }
    }
}
impl server::Handler for Peer {
    type Error = anyhow::Error;
    async fn auth_password(&mut self, user: &str, secret: &str) -> anyhow::Result<Auth> {
        Ok(
            if self.accepted_key.lock().unwrap().is_none()
                && user == "tester"
                && secret == "test-password"
            {
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
    ) -> anyhow::Result<Auth> {
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        Ok(
            if user == "tester" && self.accepted_key.lock().unwrap().as_ref() == Some(&fingerprint)
            {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> anyhow::Result<()> {
        if let Ok(mut stream) = TcpStream::connect((host, port as u16)).await {
            reply.accept().await;
            self.jobs.spawn(async move {
                let mut channel = channel.into_stream();
                let _ = copy_bidirectional(&mut stream, &mut channel).await;
            });
        }
        Ok(())
    }
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> anyhow::Result<bool> {
        let listener = TcpListener::bind((address, *port as u16)).await?;
        *port = listener.local_addr()?.port().into();
        let port = *port;
        let address = address.to_string();
        let handle = session.handle();
        self.listener = Some(tokio::spawn(async move {
            let mut jobs = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = jobs.join_next(), if !jobs.is_empty() => {},
                    result = listener.accept() => {
                        let Ok((mut stream, origin)) = result else { break };
                        let handle = handle.clone(); let address = address.clone();
                        jobs.spawn(async move {
                            if let Ok(channel) = handle.channel_open_forwarded_tcpip(address, port, origin.ip().to_string(), origin.port().into()).await {
                                let mut channel = channel.into_stream(); let _ = copy_bidirectional(&mut stream, &mut channel).await;
                            }
                        });
                    }
                }
            }
        }));
        Ok(true)
    }
    async fn cancel_tcpip_forward(
        &mut self,
        _: &str,
        _: u32,
        _: &mut server::Session,
    ) -> anyhow::Result<bool> {
        if let Some(task) = self.listener.take() {
            task.abort();
            let _ = task.await;
        }
        self.cancelled.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
struct Fixture {
    port: u16,
    fingerprint: String,
    state: Server,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture() -> Fixture {
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Server::default();
    let mut server = state.clone();
    let task = tokio::spawn(async move {
        server.run_on_socket(config, &listener).await.unwrap();
    });
    Fixture {
        port,
        fingerprint,
        state,
        task,
    }
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
fn rule(kind: Kind, port: u16, destination: u16) -> Forward {
    Forward {
        id: "rule".into(),
        label: "Tunnel".into(),
        kind,
        host_id: "test".into(),
        bind_port: port,
        destination_host: "127.0.0.1".into(),
        destination_port: destination,
        ..Default::default()
    }
}
async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
async fn wait_phase(r: &RunningRule, phase: Phase) {
    timeout(Duration::from_secs(10), async {
        loop {
            let s = r.status();
            if s.phase == phase {
                break;
            }
            assert_ne!(s.phase, Phase::Error, "{}", s.message);
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn wait_zero(count: &AtomicUsize) {
    timeout(Duration::from_secs(5), async {
        while count.load(Ordering::SeqCst) != 0 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn exercise(kind: Kind, key: Option<opentermius::keychain::KeyEntry>) {
    let fixture = fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let mut store = opentermius::store::Store::open(directory.path().join("config.json")).unwrap();
    let mut host = profile(fixture.port);
    let material = if let Some(entry) = key {
        *fixture.state.accepted_key.lock().unwrap() = Some(entry.fingerprint.clone());
        host.auth_type = "key".into();
        host.key_id = entry.id.clone();
        store.add_key(entry).unwrap();
        store.save(host.clone(), None, false).unwrap();
        // Load the persisted entry exactly as forwarding startup does, with no key file.
        let reopened = opentermius::store::Store::open(store.path.clone()).unwrap();
        Some(reopened.key_material(&host.key_id).unwrap())
    } else {
        None
    };
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = echo.local_addr().unwrap().port();
    let active = Arc::new(AtomicUsize::new(0));
    let count = active.clone();
    let echo_task = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        loop {
            tokio::select! {
                Some(_) = jobs.join_next(), if !jobs.is_empty() => {},
                result = echo.accept() => {
                    let (stream, _) = result.unwrap(); count.fetch_add(1, Ordering::SeqCst); let guard = Count(count.clone());
                    jobs.spawn(async move { let _guard = guard; let (mut read, mut write) = stream.into_split(); let _ = tokio::io::copy(&mut read, &mut write).await; });
                }
            }
        }
    });
    let port = free_port().await;
    let mut running = RunningRule::start(
        &tokio::runtime::Handle::current(),
        rule(kind, port, destination),
        host,
        Zeroizing::new(if material.is_some() {
            String::new()
        } else {
            "test-password".into()
        }),
        material,
        fixture.fingerprint.clone(),
    )
    .unwrap();
    wait_phase(&running, Phase::Running).await;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    if kind == Kind::Dynamic {
        stream.write_all(&[5, 1, 0]).await.unwrap();
        let mut auth = [0; 2];
        stream.read_exact(&mut auth).await.unwrap();
        assert_eq!(auth, [5, 0]);
        // Domain requests are resolved by the SSH server.
        let mut request = vec![5, 1, 0, 3, 9];
        request.extend_from_slice(b"localhost");
        request.extend_from_slice(&destination.to_be_bytes());
        stream.write_all(&request).await.unwrap();
        let mut reply = [0; 10];
        timeout(Duration::from_secs(5), stream.read_exact(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply[1], 0);
    }
    let payload = vec![b'x'; 128 * 1024];
    let (mut read, mut write) = stream.split();
    let mut echoed = vec![0; payload.len()];
    timeout(Duration::from_secs(10), async {
        tokio::try_join!(write.write_all(&payload), read.read_exact(&mut echoed)).unwrap();
    })
    .await
    .unwrap();
    assert_eq!(echoed, payload);
    timeout(Duration::from_secs(2), async {
        while running.status().active_connections != 1 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Stop while a channel is open. All pumps must end, including the remote echo.
    timeout(Duration::from_secs(10), running.stop())
        .await
        .unwrap();
    assert_eq!(running.status().phase, Phase::Stopped);
    assert_eq!(running.status().active_connections, 0);
    let rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    drop(rebound);
    let mut byte = [0];
    let closed = timeout(Duration::from_secs(3), stream.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
    wait_zero(&active).await;
    wait_zero(&fixture.state.peers).await;
    if kind == Kind::Remote {
        assert_eq!(fixture.state.cancelled.load(Ordering::SeqCst), 1);
    }
    echo_task.abort();
    let _ = echo_task.await;
}
#[tokio::test]
async fn local_forwarding_and_stop_cleanup() {
    exercise(Kind::Local, None).await;
}
#[tokio::test]
async fn dynamic_socks5_end_to_end_and_cleanup() {
    exercise(Kind::Dynamic, None).await;
}
#[tokio::test]
async fn remote_forwarding_and_cancel_cleanup() {
    exercise(Kind::Remote, None).await;
}

#[cfg(windows)]
#[tokio::test]
async fn keychain_key_authenticates_all_forwarding_modes_without_a_file_or_password() {
    let entry =
        opentermius::keychain::generate("forwarding key", opentermius::keychain::KeyKind::Ed25519)
            .unwrap();
    for kind in [Kind::Local, Kind::Remote, Kind::Dynamic] {
        exercise(kind, Some(entry.clone())).await;
    }
}

#[test]
fn validation_and_serialization() {
    let mut f = rule(Kind::Local, 8080, 22);
    f.label = "  Tunnel  ".into();
    f.id.clear();
    f.validate(&[]).unwrap();
    assert!(!f.id.is_empty());
    assert_eq!(f.label, "Tunnel");
    let mut duplicate = f.clone();
    duplicate.id.clear();
    duplicate.label = "tunnel".into();
    assert!(duplicate.validate(&[f.clone()]).is_err());
    f.validate(&[f.clone()]).unwrap();
    for invalid in [
        Forward {
            bind_port: 0,
            ..f.clone()
        },
        Forward {
            destination_port: 0,
            ..f.clone()
        },
        Forward {
            bind_address: "localhost".into(),
            ..f.clone()
        },
        Forward {
            destination_host: "".into(),
            ..f.clone()
        },
        Forward {
            host_id: "".into(),
            ..f.clone()
        },
        Forward {
            label: "\n".into(),
            ..f.clone()
        },
    ] {
        assert!(invalid.clone().validate(&[]).is_err());
    }
    let mut dynamic = Forward {
        kind: Kind::Dynamic,
        bind_address: "::1".into(),
        destination_host: "".into(),
        destination_port: 0,
        ..f.clone()
    };
    dynamic.validate(&[]).unwrap();
    let serialized = serde_json::to_string(&dynamic).unwrap();
    assert!(serialized.contains("bindAddress"));
    assert_eq!(
        serde_json::from_str::<Forward>(&serialized).unwrap().kind,
        Kind::Dynamic
    );
    assert_eq!(
        serde_json::from_str::<Forward>("{\"id\":\"old\"}")
            .unwrap()
            .bind_address,
        "127.0.0.1"
    );
    assert!(serde_json::from_str::<Forward>("{\"bindPort\":65536}").is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut store = opentermius::store::Store::open(path.clone()).unwrap();
    dynamic.auto_start = true;
    store.data.forwards.push(dynamic.clone());
    store.persist().unwrap();
    let saved = opentermius::store::Store::open(path)
        .unwrap()
        .data
        .forwards
        .remove(0);
    assert_eq!(saved.label, dynamic.label);
    assert_eq!(saved.kind, Kind::Dynamic);
    assert!(saved.auto_start);
}
async fn parse(bytes: Vec<u8>) -> (anyhow::Result<(String, u16)>, Vec<u8>) {
    let (mut client, mut server) = tokio::io::duplex(1024);
    let task = tokio::spawn(async move { socks5_connect(&mut server).await });
    client.write_all(&bytes).await.unwrap();
    client.shutdown().await.unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.unwrap();
    (task.await.unwrap(), reply)
}
#[tokio::test]
async fn socks5_ipv4_ipv6_and_domain() {
    for (mut request, expected) in [
        (vec![1, 127, 0, 0, 1], "127.0.0.1"),
        (vec![3, 3, b'f', b'o', b'o'], "foo"),
        (
            {
                let mut v = vec![4];
                v.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
                v
            },
            "::1",
        ),
    ] {
        let mut bytes = vec![5, 2, 2, 0, 5, 1, 0];
        bytes.append(&mut request);
        bytes.extend_from_slice(&443u16.to_be_bytes());
        let (parsed, reply) = parse(bytes).await;
        assert_eq!(parsed.unwrap(), (expected.into(), 443));
        assert_eq!(reply, [5, 0]);
    }
}
#[tokio::test]
async fn socks5_rejections_and_malformed_input() {
    for (request, code) in [
        (vec![5, 2, 0, 1], 7),
        (vec![5, 3, 0, 1], 7),
        (vec![5, 1, 1, 1], 1),
        (vec![4, 1, 0, 1], 1),
        (vec![5, 1, 0, 9], 8),
        (vec![5, 1, 0, 3, 0], 4),
        (vec![5, 1, 0, 3, 1, 255], 4),
        (vec![5, 1, 0, 1, 127, 0, 0, 1, 0, 0], 1),
    ] {
        let mut bytes = vec![5, 1, 0];
        bytes.extend(request);
        let (result, reply) = parse(bytes).await;
        assert!(result.is_err());
        assert_eq!(reply.len(), 12);
        assert_eq!(reply[3], code);
    }
    for bytes in [
        vec![],
        vec![5],
        vec![4, 1, 0],
        vec![5, 1],
        vec![5, 1, 0, 5, 1],
        vec![5, 1, 0, 5, 1, 0, 4, 0],
    ] {
        assert!(parse(bytes).await.0.is_err());
    }
    for bytes in [vec![5, 0], vec![5, 1, 2]] {
        let (result, reply) = parse(bytes).await;
        assert!(result.is_err());
        assert_eq!(reply, [5, 255]);
    }
}
#[tokio::test]
async fn untrusted_fingerprint_and_cancel_during_handshake() {
    let f = fixture().await;
    let port = free_port().await;
    assert!(
        RunningRule::start(
            &tokio::runtime::Handle::current(),
            rule(Kind::Local, port, 22),
            profile(f.port),
            Zeroizing::new(String::new()),
            None,
            "".into()
        )
        .is_err()
    );
    let mut bad = RunningRule::start(
        &tokio::runtime::Handle::current(),
        rule(Kind::Local, port, 22),
        profile(f.port),
        Zeroizing::new("test-password".into()),
        None,
        "SHA256:incorrect".into(),
    )
    .unwrap();
    timeout(Duration::from_secs(5), async {
        while bad.status().phase != Phase::Error {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(bad.status().message.contains("主机密钥已变化"));
    bad.stop().await;
    // A TCP server that never sends an SSH banner must not delay Stop.
    let stalled = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ssh_port = stalled.local_addr().unwrap().port();
    let mut running = RunningRule::start(
        &tokio::runtime::Handle::current(),
        rule(Kind::Local, port, 22),
        profile(ssh_port),
        Zeroizing::new("test-password".into()),
        None,
        f.fingerprint.clone(),
    )
    .unwrap();
    let (_socket, _) = timeout(Duration::from_secs(3), stalled.accept())
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), running.stop())
        .await
        .unwrap();
    assert_eq!(running.status().phase, Phase::Stopped);
    TcpListener::bind(("127.0.0.1", port)).await.unwrap();
}

#[tokio::test]
async fn dynamic_connection_limit_and_pending_handshake_cleanup() {
    let f = fixture().await;
    let port = free_port().await;
    let mut running = RunningRule::start(
        &tokio::runtime::Handle::current(),
        rule(Kind::Dynamic, port, 0),
        profile(f.port),
        Zeroizing::new("test-password".into()),
        None,
        f.fingerprint.clone(),
    )
    .unwrap();
    wait_phase(&running, Phase::Running).await;
    let mut clients = Vec::new();
    for _ in 0..opentermius::forwarding::MAX_CONNECTIONS {
        clients.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
    }
    timeout(Duration::from_secs(5), async {
        while running.status().active_connections != opentermius::forwarding::MAX_CONNECTIONS {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut excess = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut byte = [0];
    let closed = timeout(Duration::from_secs(3), excess.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
    timeout(Duration::from_secs(5), running.stop())
        .await
        .unwrap();
    assert_eq!(running.status().active_connections, 0);
    for mut client in clients {
        let closed = timeout(Duration::from_secs(2), client.read(&mut byte))
            .await
            .unwrap();
        assert!(closed.is_err() || closed.unwrap() == 0);
    }
    TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    wait_zero(&f.state.peers).await;
}

#[tokio::test]
async fn listener_conflict_is_reported_and_ssh_is_closed() {
    let f = fixture().await;
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    let mut running = RunningRule::start(
        &tokio::runtime::Handle::current(),
        rule(Kind::Local, port, 22),
        profile(f.port),
        Zeroizing::new("test-password".into()),
        None,
        f.fingerprint.clone(),
    )
    .unwrap();
    timeout(Duration::from_secs(5), async {
        while running.status().phase != Phase::Error {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(running.status().message.contains("无法监听"));
    running.stop().await;
    wait_zero(&f.state.peers).await;
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires an interactive Windows desktop"]
async fn native_forwarding_autostart_and_exit_cleanup() {
    exercise_autostart(false).await;
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires an interactive Windows desktop"]
async fn native_forwarding_keychain_autostart_and_exit_cleanup() {
    exercise_autostart(true).await;
}

#[cfg(windows)]
async fn exercise_autostart(use_keychain: bool) {
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let fixture = fixture().await;
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = echo.local_addr().unwrap().port();
    let echo_task = tokio::spawn(async move {
        let (stream, _) = echo.accept().await.unwrap();
        let (mut read, mut write) = stream.into_split();
        let _ = tokio::io::copy(&mut read, &mut write).await;
    });
    let directory = tempfile::tempdir().unwrap();
    let mut store =
        opentermius::store::Store::open(directory.path().join("opentermius-native.json")).unwrap();
    let mut host = profile(fixture.port);
    if use_keychain {
        let entry = opentermius::keychain::generate(
            "autostart key",
            opentermius::keychain::KeyKind::Ed25519,
        )
        .unwrap();
        *fixture.state.accepted_key.lock().unwrap() = Some(entry.fingerprint.clone());
        host.auth_type = "key".into();
        host.key_id = entry.id.clone();
        store.add_key(entry).unwrap();
        // A stale saved credential must not interfere with keychain authentication.
        host.encrypted_secret = Some("invalid-old-credential".into());
    } else {
        host.encrypted_secret = Some(opentermius::store::protect("test-password").unwrap());
    }
    store.data.hosts.push(host);
    store
        .trust("127.0.0.1", fixture.port, &fixture.fingerprint)
        .unwrap();
    let port = free_port().await;
    let mut forward = rule(Kind::Local, port, destination);
    forward.auto_start = true;
    store.data.forwards.push(forward);
    store.persist().unwrap();
    let executable = std::env::var_os("OPENTERMIUS_TEST_EXECUTABLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_opentermius").into());
    let mut child = ChildGuard(
        std::process::Command::new(executable)
            .arg("--smoke-test")
            .arg("--smoke-scene=section-forwarding")
            .arg(format!("--data-dir={}", directory.path().display()))
            .arg(format!(
                "--output={}",
                directory.path().join("output").display()
            ))
            .spawn()
            .unwrap(),
    );
    let mut stream = timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)).await {
                break stream;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "application exited before auto-starting the rule"
            );
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stream.write_all(b"autostart").await.unwrap();
    let mut bytes = [0; 9];
    timeout(Duration::from_secs(3), stream.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"autostart");
    let exit = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(exit.success());
    assert!(directory.path().join("output/native-window.png").exists());
    TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    timeout(Duration::from_secs(3), echo_task)
        .await
        .unwrap()
        .unwrap();
    wait_zero(&fixture.state.peers).await;
}
