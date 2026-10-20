//! Restored credentials must authenticate, not merely deserialize successfully.
#[cfg(windows)]
mod windows {
    use opentermius::{
        forwarding::{Forward, Phase, RunningRule},
        keychain::{self, KeyKind},
        logs::{self, LogBook},
        recovery::{self, RestoreMode},
        store::{self, Store},
    };
    use russh::{
        Channel,
        server::{self, Auth, Server as _},
    };
    use serde_json::json;
    use std::{fs, sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
        net::{TcpListener, TcpStream},
        task::{JoinHandle, JoinSet},
        time::{sleep, timeout},
    };
    use zeroize::Zeroizing;

    #[derive(Clone)]
    struct Server {
        fingerprint: String,
    }
    impl server::Server for Server {
        type Handler = Peer;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
            Peer {
                fingerprint: self.fingerprint.clone(),
                jobs: JoinSet::new(),
            }
        }
    }
    struct Peer {
        fingerprint: String,
        jobs: JoinSet<()>,
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
            // The fixture is deliberately restricted to local test endpoints.
            anyhow::ensure!(host == "127.0.0.1");
            let mut stream = TcpStream::connect((host, port as u16)).await?;
            reply.accept().await;
            self.jobs.spawn(async move {
                let mut channel = channel.into_stream();
                let _ = copy_bidirectional(&mut stream, &mut channel).await;
            });
            Ok(())
        }
    }
    struct Task(JoinHandle<()>);
    impl Drop for Task {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn phase(rule: &RunningRule, expected: Phase) {
        timeout(Duration::from_secs(10), async {
            loop {
                let status = rule.status();
                assert_ne!(status.phase, Phase::Error, "{}", status.message);
                if status.phase == expected {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("forwarding state timed out");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovered_keychain_and_file_keys_forward_bytes_and_log_lifecycle() {
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
        let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = echo.local_addr().unwrap().port();
        let _echo = Task(tokio::spawn(async move {
            let mut jobs = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = jobs.join_next(), if !jobs.is_empty() => {},
                    result = echo.accept() => {
                        let (mut stream, _) = result.unwrap();
                        jobs.spawn(async move {
                            let (mut read, mut write) = stream.split();
                            let _ = tokio::io::copy(&mut read, &mut write).await;
                        });
                    }
                }
            }
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
        fs::remove_file(key_path).unwrap();
        for mode in [RestoreMode::Merge, RestoreMode::Replace] {
            let temp = tempfile::tempdir().unwrap();
            let mut store = Store::open(temp.path().join("config.json")).unwrap();
            recovery::restore(&mut store, &code, None, mode).unwrap();
            let store = Store::open(store.path.clone()).unwrap();
            let mut book = LogBook::open(temp.path()).unwrap();
            for host in &store.data.hosts {
                let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let bind_port = reserved.local_addr().unwrap().port();
                drop(reserved);
                let rule = Forward {
                    label: host.name.clone(),
                    host_id: host.id.clone(),
                    bind_port,
                    destination_host: "127.0.0.1".into(),
                    destination_port: destination,
                    ..Default::default()
                };
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
                let mut running = RunningRule::start(
                    &tokio::runtime::Handle::current(),
                    rule,
                    host.clone(),
                    Zeroizing::new(String::new()),
                    material,
                    store.data.known_hosts[&store::key(&host.host, host.port)].clone(),
                )
                .unwrap();
                phase(&running, Phase::Running).await;
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
                timeout(Duration::from_secs(10), async {
                    let mut stream = TcpStream::connect(("127.0.0.1", bind_port)).await.unwrap();
                    let bytes: Vec<u8> = (0..=255).cycle().take(32 * 1024).collect();
                    let mut received = vec![0; bytes.len()];
                    let (mut read, mut write) = stream.split();
                    tokio::try_join!(write.write_all(&bytes), read.read_exact(&mut received))
                        .unwrap();
                    assert_eq!(received, bytes);
                })
                .await
                .unwrap();
                timeout(Duration::from_secs(10), running.stop())
                    .await
                    .unwrap();
                assert_eq!(running.status().phase, Phase::Stopped);
                assert_eq!(running.status().active_connections, 0);
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
