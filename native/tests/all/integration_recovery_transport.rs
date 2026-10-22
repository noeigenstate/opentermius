//! Restored credentials must authenticate, not merely deserialize successfully.
#[cfg(windows)]
mod windows {
    use opentermius::{
        keychain::{self, KeyKind},
        logs::{self, LogBook},
        recovery::{self, RestoreMode},
        ssh::{self, Event},
        store::{self, Store},
    };
    use russh::{
        Channel, ChannelId,
        server::{self, Auth, Server as _},
    };
    use serde_json::json;
    use std::{collections::HashMap, fs, sync::Arc, time::Duration};
    use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle, time::timeout};

    #[derive(Clone)]
    struct Server {
        fingerprint: String,
    }
    impl server::Server for Server {
        type Handler = Peer;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
            Peer {
                fingerprint: self.fingerprint.clone(),
                channels: HashMap::new(),
            }
        }
    }
    struct Peer {
        fingerprint: String,
        channels: HashMap<ChannelId, Channel<server::Msg>>,
    }
    impl server::Handler for Peer {
        type Error = anyhow::Error;
        async fn auth_publickey(
            &mut self,
            user: &str,
            key: &russh::keys::PublicKey,
        ) -> anyhow::Result<Auth> {
            Ok(
                if user == "tester"
                    && key.fingerprint(russh::keys::HashAlg::Sha256).to_string() == self.fingerprint
                {
                    Auth::Accept
                } else {
                    Auth::reject()
                },
            )
        }
        async fn channel_open_session(
            &mut self,
            channel: Channel<server::Msg>,
            reply: server::ChannelOpenHandle,
            _: &mut server::Session,
        ) -> anyhow::Result<()> {
            self.channels.insert(channel.id(), channel);
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
            s: &mut server::Session,
        ) -> anyhow::Result<()> {
            s.channel_success(id)?;
            Ok(())
        }
        async fn shell_request(&mut self, id: ChannelId, s: &mut server::Session) -> anyhow::Result<()> {
            s.channel_success(id)?;
            s.data(id, b"restored-login-ok\r\n".to_vec())?;
            Ok(())
        }
    }
    struct Task(JoinHandle<()>);
    impl Drop for Task {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    /// Waits for the terminal to connect and show the server's greeting.
    async fn logged_in(rx: &mut mpsc::Receiver<Event>) {
        timeout(Duration::from_secs(10), async {
            let mut connected = false;
            let mut output = Vec::new();
            loop {
                match rx.recv().await.expect("session ended") {
                    Event::Status { connected: true, .. } => connected = true,
                    Event::Status { error: true, message, .. } => panic!("{message}"),
                    Event::Output { data, .. } => {
                        output.extend(data);
                        if connected && String::from_utf8_lossy(&output).contains("restored-login-ok") { break; }
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("restored host did not log in");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovered_keychain_and_file_keys_authenticate_and_log_lifecycle() {
        let entry = keychain::generate("restored authentication", KeyKind::Ed25519).unwrap();
        let server_key = russh::keys::PrivateKey::random(
            &mut russh::keys::key::safe_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let server_fingerprint = server_key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        let config = Arc::new(server::Config {
            keys: vec![server_key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ssh_port = listener.local_addr().unwrap().port();
        let mut server = Server {
            fingerprint: entry.fingerprint.clone(),
        };
        let _server = Task(tokio::spawn(async move {
            server.run_on_socket(config, &listener).await.unwrap();
        }));
        let source_dir = tempfile::tempdir().unwrap();
        let key_path = source_dir.path().join("source.key");
        fs::write(&key_path, entry.export_private().unwrap().as_bytes()).unwrap();
        let mut source = Store::open(source_dir.path().join("config.json")).unwrap();
        source.add_key(entry.clone()).unwrap();
        for (id, key_id, path) in [
            ("keychain-host", entry.id.as_str(), String::new()),
            ("file-host", "", key_path.to_string_lossy().into_owned()),
        ] {
            source.save(serde_json::from_value(json!({
                "id": id, "name": id, "host": "127.0.0.1", "port": ssh_port,
                "username": "tester", "authType": "key", "keyId": key_id, "privateKeyPath": path
            })).unwrap(), None, false).unwrap();
        }
        source
            .trust("127.0.0.1", ssh_port, &server_fingerprint)
            .unwrap();
        let code = recovery::generate(&source.data, None, true).unwrap();
        // The file key must come back from the recovery code, not from the old path.
        fs::remove_file(key_path).unwrap();
        for mode in [RestoreMode::Merge, RestoreMode::Replace] {
            let temp = tempfile::tempdir().unwrap();
            let mut store = Store::open(temp.path().join("config.json")).unwrap();
            recovery::restore(&mut store, &code, None, mode).unwrap();
            let store = Store::open(store.path.clone()).unwrap();
            let mut book = LogBook::open(temp.path()).unwrap();
            for host in &store.data.hosts {
                let id = book
                    .start(
                        &host.id,
                        &host.name,
                        &format!("127.0.0.1:{ssh_port}"),
                        "ssh",
                    )
                    .unwrap();
                assert_eq!(
                    LogBook::open(temp.path())
                        .unwrap()
                        .entries()
                        .last()
                        .unwrap()
                        .status,
                    logs::CONNECTING
                );
                let material = if host.key_id.is_empty() {
                    None
                } else {
                    Some(store.key_material(&host.key_id).unwrap())
                };
                let (tx, mut rx) = mpsc::channel(64);
                let session = ssh::spawn_with_key_material(
                    &tokio::runtime::Handle::current(),
                    host.id.clone(),
                    host.clone(),
                    String::new(),
                    material,
                    Some(store.data.known_hosts[&store::key(&host.host, host.port)].clone()),
                    tx,
                );
                logged_in(&mut rx).await;
                // Drive the public log lifecycle API after the observed SSH state.
                book.update(&id, logs::CONNECTED, None).unwrap();
                assert_eq!(
                    LogBook::open(temp.path())
                        .unwrap()
                        .entries()
                        .last()
                        .unwrap()
                        .status,
                    logs::CONNECTED
                );
                let _ = session.commands.send(ssh::Command::Disconnect).await;
                session.task.abort();
                book.finish(&id).unwrap();
                let reopened = LogBook::open(temp.path()).unwrap();
                let logged = reopened.entries().last().unwrap();
                assert_eq!(logged.status, logs::CLOSED);
                assert_eq!(logged.host_id, host.id);
                assert!(logged.ended_at.is_some());
                assert!(logged.error.is_none());
            }
            let raw = fs::read_to_string(book.path()).unwrap();
            assert_eq!(raw.lines().count(), 2);
            assert!(!raw.contains("PRIVATE KEY"));
            assert!(!raw.contains(&entry.encrypted_private_key));
        }
    }
}
