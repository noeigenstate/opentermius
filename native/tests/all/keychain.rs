//! Loopback SSH test for keychain-backed public-key authentication.
#![cfg(windows)]
use opentermius::{
    keychain::{self, KeyKind},
    ssh::{self, Command, Event},
    store::{Profile, Store},
};
use russh::{
    Channel, ChannelId,
    server::{self, Auth, Server as _},
};
use std::{
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

#[derive(Clone, Default)]
struct State {
    expected: Arc<Mutex<Option<String>>>,
    authenticated: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct Server(State);

impl server::Server for Server {
    type Handler = Peer;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
        Peer {
            state: self.0.clone(),
        }
    }
}

struct Peer {
    state: State,
}

impl server::Handler for Peer {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let expected = self.state.expected.lock().unwrap().clone();
        if user == "tester" && expected.as_deref() == Some(fingerprint.as_str()) {
            self.state.authenticated.fetch_add(1, Ordering::SeqCst);
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)?;
        session.data(id, b"keychain-auth-ok\r\n".to_vec())?;
        Ok(())
    }

    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(id, data.to_vec())?;
        Ok(())
    }
}

async fn fixture(expected: String) -> (u16, State, String, tokio::task::JoinHandle<()>) {
    let key = russh::keys::PrivateKey::random(
        &mut russh::keys::key::safe_rng(),
        russh::keys::Algorithm::Ed25519,
    )
    .unwrap();
    let host_fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let socket = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let state = State {
        expected: Arc::new(Mutex::new(Some(expected))),
        ..Default::default()
    };
    let mut server = Server(state.clone());
    let task = tokio::spawn(async move {
        server.run_on_socket(config, &socket).await.unwrap();
    });
    (port, state, host_fingerprint, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keychain_key_authenticates_without_a_file_or_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("data.json")).unwrap();
    let entry = keychain::generate("loopback", KeyKind::Ed25519).unwrap();
    let fingerprint = entry.fingerprint.clone();
    let key_id = entry.id.clone();
    store.add_key(entry).unwrap();
    store
        .save(
            Profile {
                id: "keychain-auth".into(),
                name: "loopback".into(),
                host: "127.0.0.1".into(),
                port: 22,
                username: "tester".into(),
                auth_type: "key".into(),
                private_key_path: String::new(),
                key_id: key_id.clone(),
                group: String::new(),
                encrypted_secret: None,
                tags: Vec::new(),
                last_connected: None,
                created_at: None,
            },
            None,
            false,
        )
        .unwrap();
    let profile = store.data.hosts[0].clone();
    let material = store.key_material(&key_id).unwrap();

    let (port, state, host_fingerprint, server) = fixture(fingerprint).await;
    let mut profile = profile;
    profile.port = port;
    let (tx, mut rx) = mpsc::channel(128);
    let session = ssh::spawn_with_key_material_and_options(
        &tokio::runtime::Handle::current(),
        "keychain-auth".into(),
        profile,
        String::new(),
        Some(material),
        Some(host_fingerprint),
        tx,
        ssh::ConnectOptions {
            keepalive_seconds: 0,
            timeout_seconds: 5,
            ..Default::default()
        },
    );
    timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Some(Event::Status {
                    connected: true, ..
                }) => break,
                Some(Event::Status {
                    error: true,
                    message,
                    ..
                }) => panic!("{message}"),
                Some(_) => {}
                None => panic!("session events closed"),
            }
        }
    })
    .await
    .expect("keychain SSH authentication timed out");
    assert_eq!(state.authenticated.load(Ordering::SeqCst), 1);
    let _ = session.commands.send(Command::Disconnect).await;
    let _ = timeout(Duration::from_secs(2), session.task).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_keychain_material_does_not_fall_back_to_a_file() {
    let temp = tempfile::tempdir().unwrap();
    let entry = keychain::generate("fallback test", KeyKind::Ed25519).unwrap();
    let path = temp.path().join("key.pem");
    std::fs::write(&path, entry.export_private().unwrap().as_bytes()).unwrap();
    let (port, state, host_fingerprint, server) = fixture(entry.fingerprint).await;
    let (tx, mut rx) = mpsc::channel(128);
    let session = ssh::spawn(
        &tokio::runtime::Handle::current(),
        "missing-material".into(),
        Profile {
            id: "missing-material".into(),
            name: "fallback test".into(),
            host: "127.0.0.1".into(),
            port,
            username: "tester".into(),
            auth_type: "key".into(),
            private_key_path: path.to_str().unwrap().into(),
            key_id: entry.id,
            group: String::new(),
            encrypted_secret: None,
            tags: Vec::new(),
            last_connected: None,
            created_at: None,
        },
        String::new(),
        Some(host_fingerprint),
        tx,
    );
    timeout(Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Some(Event::Status {
                    connected: true, ..
                }) => panic!("used the file key"),
                Some(Event::Status {
                    error: true,
                    message,
                    ..
                }) => {
                    assert!(message.contains("Keychain"));
                    break;
                }
                Some(_) => {}
                None => panic!("session events closed without an error"),
            }
        }
    })
    .await
    .expect("missing keychain material was not rejected");
    assert_eq!(state.authenticated.load(Ordering::SeqCst), 0);
    timeout(Duration::from_secs(2), session.task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}
