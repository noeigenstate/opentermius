//! Adversarial regression checks; all credentials and servers are local fixtures.
use opentermius::{
    forwarding::{self, Forward},
    import_xshell, keychain,
    recovery::{self, RestoreMode},
    snippets, ssh,
    store::{self, Data, Profile, Store},
    terminal::Terminal,
};
use std::{fs, path::Path};

fn profile() -> Profile {
    store::parse_quick_connect("tester@example.invalid").unwrap()
}

#[test]
fn windows_download_names_reject_device_aliases_and_path_escapes() {
    for name in [
        "..",
        ".",
        "../escape",
        "a/../../escape",
        "/absolute",
        "C:\\escape",
        "\\\\server\\share",
        "file:stream",
        "CON",
        "nul.txt",
        "COM1.log",
        "LPT9",
        "COM¹.txt",
        "LPT²",
        "COM³",
        "CONIN$",
        "CONOUT$",
        "a.",
        "a ",
        "bad\0name",
    ] {
        assert!(
            !ssh::safe_local_component(name),
            "accepted unsafe name: {name:?}"
        );
    }
    for name in [
        "normal.txt",
        "文件.txt",
        "com10.txt",
        "console",
        "report..txt",
    ] {
        assert!(ssh::safe_local_component(name));
    }
}

#[test]
fn terminal_input_and_screen_debug_do_not_disclose_secrets() {
    let secret = "security-regression-secret";
    let input = ssh::Command::Input(secret.as_bytes().to_vec());
    assert!(!format!("{input:?}").contains(&format!("{:?}", secret.as_bytes())));
    let mut terminal = Terminal::new(24, 80);
    terminal.feed(secret.as_bytes());
    let event = ssh::Event::Screen {
        id: "test".into(),
        snapshot: terminal.snapshot(),
    };
    assert!(!format!("{event:?}").contains(secret));
    let material = keychain::KeyMaterial {
        pem: zeroize::Zeroizing::new(secret.to_owned()),
        passphrase: Some(zeroize::Zeroizing::new(secret.to_owned())),
    };
    assert!(!format!("{material:?}").contains(secret));
}

#[test]
fn malformed_private_key_errors_do_not_echo_input() {
    let secret = "private-key-parser-secret";
    let text = format!("PuTTY-User-Key-File-3: ssh-ed25519\n{secret}\n");
    let error = keychain::decode_private_key(&text, Some(secret)).unwrap_err();
    assert!(!format!("{error:#}").contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}

#[test]
fn forwarding_rule_count_is_bounded_but_existing_rules_can_be_edited() {
    let existing: Vec<_> = (0..64)
        .map(|n| Forward {
            id: format!("rule-{n}"),
            label: format!("Rule {n}"),
            host_id: "host".into(),
            bind_port: 9000 + n,
            destination_host: "127.0.0.1".into(),
            destination_port: 22,
            ..Default::default()
        })
        .collect();
    let mut additional = Forward {
        id: String::new(),
        label: "Additional".into(),
        ..existing[0].clone()
    };
    assert!(additional.validate(&existing).is_err());
    let mut edit = existing[0].clone();
    edit.label = "Edited".into();
    assert!(edit.validate(&existing).is_ok());
}

#[test]
fn xshell_import_is_read_only_and_caps_recursion() {
    let temp = tempfile::tempdir().unwrap();
    let contents = "[CONNECTION]\nProtocol=SSH\nHost=example.invalid\n[CONNECTION:AUTHENTICATION]\nUserName=tester\nPassword=vendor-secret\n";
    fs::write(temp.path().join("host.xsh"), contents).unwrap();
    let mut nested = temp.path().to_path_buf();
    for _ in 0..70 {
        nested.push("d");
    }
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("deep.xsh"), contents).unwrap();
    let scan = import_xshell::scan(temp.path());
    assert_eq!(scan.hosts.len(), 1, "excessively deep session was imported");
    assert!(
        scan.skipped
            .iter()
            .any(|(_, reason)| reason.contains("too deep"))
    );
    assert!(!format!("{scan:?}").contains("vendor-secret"));
    assert_eq!(
        fs::read_to_string(temp.path().join("host.xsh")).unwrap(),
        contents
    );
    assert_eq!(
        fs::read_to_string(nested.join("deep.xsh")).unwrap(),
        contents
    );
}

#[test]
fn xshell_import_caps_total_entries() {
    let temp = tempfile::tempdir().unwrap();
    for n in 0..10_002 {
        fs::write(temp.path().join(format!("{n}.txt")), "").unwrap();
    }
    let scan = import_xshell::scan(temp.path());
    assert!(scan.hosts.is_empty());
    assert!(
        scan.skipped
            .iter()
            .any(|(_, reason)| reason.contains("Too many"))
    );
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction creation failed");
}

#[cfg(windows)]
#[test]
fn recovery_refuses_junction_keys_directory_without_external_writes() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let key = source.path().join("private.key");
    fs::write(&key, "fixture-private-key").unwrap();
    let mut host = profile();
    host.private_key_path = key.to_string_lossy().into();
    let data = Data {
        version: 2,
        hosts: vec![host],
        ..Default::default()
    };
    let code = recovery::generate(&data, None, true).unwrap();
    let keys = target.path().join("keys");
    junction(&keys, outside.path());
    let mut store = Store::open(target.path().join("config.json")).unwrap();
    let result = recovery::restore(&mut store, &code, None, RestoreMode::Merge);
    let written = fs::read_dir(outside.path()).unwrap().count();
    fs::remove_dir(&keys).unwrap();
    assert!(result.is_err(), "recovery accepted a junction");
    assert_eq!(written, 0, "recovery wrote outside the config directory");
    assert!(store.data.hosts.is_empty());
    assert!(!store.path.exists());
}

#[cfg(windows)]
#[test]
fn xshell_scan_refuses_selected_junction_root() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("host.xsh"),
        "[CONNECTION]\nProtocol=SSH\nHost=outside.invalid\n",
    )
    .unwrap();
    let link = temp.path().join("sessions");
    junction(&link, outside.path());
    let scanned = import_xshell::scan(&link);
    fs::remove_dir(&link).unwrap();
    assert!(scanned.hosts.is_empty());
    assert!(!scanned.skipped.is_empty());
}

#[cfg(windows)]
#[test]
fn saved_credentials_exports_and_recovery_respect_secret_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    let secret = "security-only-password-2026";
    store.save(profile(), Some(secret), true).unwrap();
    let entry = keychain::generate("test key", keychain::KeyKind::Ed25519).unwrap();
    let pem = entry.load().unwrap().pem;
    store.add_key(entry).unwrap();
    let config = fs::read_to_string(&store.path).unwrap();
    assert!(!config.contains(secret));
    assert!(!config.contains(pem.as_str()));
    let exported = temp.path().join("hosts.json");
    store.export(&exported).unwrap();
    let export = fs::read_to_string(exported).unwrap();
    assert!(!export.contains(secret));
    assert!(!export.contains(&store.data.keys[0].encrypted_private_key));
    for include in [false, true] {
        let code = recovery::generate(&store.data, None, include).unwrap();
        let mut restored =
            Store::open(temp.path().join(format!("restore-{include}.json"))).unwrap();
        recovery::restore(&mut restored, &code, None, RestoreMode::Merge).unwrap();
        let encrypted = &restored.data.hosts[0].encrypted_secret;
        if include {
            assert_eq!(
                store::unprotect(encrypted.as_ref().unwrap()).unwrap(),
                secret
            );
            assert_eq!(*restored.data.keys[0].load().unwrap().pem, *pem);
        } else {
            assert!(encrypted.is_none());
            assert!(restored.data.keys[0].encrypted_private_key.is_empty());
        }
        let config = fs::read_to_string(&restored.path).unwrap();
        assert!(!config.contains(secret));
        assert!(!config.contains(pem.as_str()));
    }
}

#[test]
fn known_hosts_conflicts_never_replace_existing_pins() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    store
        .trust("example.invalid", 22, "SHA256:original")
        .unwrap();
    let result = store
        .import_known_hosts(&[store::KnownHostEntry {
            host: "example.invalid".into(),
            port: 22,
            fingerprint: "SHA256:changed".into(),
        }])
        .unwrap();
    assert_eq!(result.conflicts, 1);
    assert!(
        store
            .trust("example.invalid", 22, "SHA256:changed")
            .is_err()
    );
    let reopened = Store::open(store.path).unwrap();
    assert_eq!(
        reopened.data.known_hosts[&store::key("example.invalid", 22)],
        "SHA256:original"
    );
}

#[test]
fn oversized_imports_logs_and_recovery_are_rejected_before_processing() {
    let temp = tempfile::tempdir().unwrap();
    let oversized = temp.path().join("large.json");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    assert!(store.import(&oversized).is_err());
    assert!(Store::open(oversized).is_err());
    let logs = temp.path().join(opentermius::logs::FILE_NAME);
    fs::File::create(logs)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    assert!(opentermius::logs::LogBook::open(temp.path()).is_err());
    assert!(
        recovery::restore(
            &mut store,
            &"x".repeat(20 * 1024 * 1024 + 1),
            None,
            RestoreMode::Merge
        )
        .is_err()
    );
    assert!(store.data.hosts.is_empty());
    assert!(!store.path.exists());
}

#[test]
fn malformed_recovery_errors_do_not_echo_payload_secrets() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ring::aead;
    let secret = "recovery-parser-secret";
    let mut data = serde_json::to_value(Data {
        version: 2,
        ..Default::default()
    })
    .unwrap();
    data["settings"]["appearance"] = serde_json::json!(secret);
    let payload = serde_json::json!({"data": data, "keyFiles": {}, "emptySecrets": []});
    let mut compressed =
        miniz_oxide::deflate::compress_to_vec_zlib(&serde_json::to_vec(&payload).unwrap(), 6);
    let mut header = vec![1, 0, 1];
    header.extend_from_slice(&[0; 32]);
    header.extend_from_slice(&[0; 12]);
    let cipher =
        aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &[0; 32]).unwrap());
    cipher
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key([0; 12]),
            aead::Aad::from(&header),
            &mut compressed,
        )
        .unwrap();
    header.extend(compressed);
    let code = format!("OTR1-{}", URL_SAFE_NO_PAD.encode(header));
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    let error = recovery::restore(&mut store, &code, None, RestoreMode::Merge).unwrap_err();
    assert!(!format!("{error:#}").contains(secret));
    assert!(!format!("{error:?}").contains(secret));
    assert!(!store.path.exists());
}

#[test]
fn bracketed_paste_cannot_break_out_and_query_replies_are_bounded() {
    let bytes = snippets::to_terminal_input("a\x1b[201~\necho injected\x1b[200~", true);
    assert!(bytes.starts_with(b"\x1b[200~"));
    assert!(bytes.ends_with(b"\x1b[201~\r"));
    assert_eq!(bytes.iter().filter(|b| **b == 27).count(), 2);
    let mut terminal = Terminal::new(u16::MAX, u16::MAX);
    for _ in 0..100_000 {
        terminal.feed(b"\x1b[6n\x1b[5n\x1b]10;?\x07");
    }
    assert!(terminal.take_responses().len() <= 65_536);
    assert!(terminal.take_responses().is_empty());
}

#[test]
fn quick_connect_has_no_shell_side_effects_and_forward_defaults_are_loopback() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("must-not-exist");
    for command in [
        format!("ssh host; echo secret > {}", marker.display()),
        format!("ssh $(echo secret > {})", marker.display()),
        "ssh -oProxyCommand=evil host".into(),
    ] {
        assert!(store::parse_quick_connect(&command).is_err());
    }
    assert!(!marker.exists());
    let rule: Forward = serde_json::from_str("{}").unwrap();
    assert_eq!(rule.bind_address, "127.0.0.1");
    let explicit: Forward = serde_json::from_str(r#"{"bindAddress":"0.0.0.0"}"#).unwrap();
    assert_eq!(explicit.bind_address, "0.0.0.0");
}

#[tokio::test]
async fn malformed_socks5_inputs_return_errors_without_panics() {
    use tokio::io::{AsyncWriteExt, duplex};
    let valid = [5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1, 0, 22];
    let mut cases: Vec<Vec<u8>> = (0..valid.len()).map(|n| valid[..n].to_vec()).collect();
    for index in [0, 3, 4, 5, 6] {
        let mut bytes = valid.to_vec();
        bytes[index] = 255;
        cases.push(bytes);
    }
    cases.extend([
        vec![5, 0],
        vec![5, 1, 0, 5, 1, 0, 3, 0, 0, 22],
        vec![5, 1, 0, 5, 1, 0, 3, 1, 0xff, 0, 22],
    ]);
    for bytes in cases {
        let (mut writer, mut parser) = duplex(1024);
        writer.write_all(&bytes).await.unwrap();
        writer.shutdown().await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            forwarding::socks5_connect(&mut parser),
        )
        .await
        .unwrap();
        assert!(result.is_err());
    }
}

mod loopback {
    use super::*;
    use russh::{
        Channel, ChannelId,
        server::{self, Auth, Server as _},
    };
    use russh_sftp::protocol::{Attrs, FileAttributes, Handle, OpenFlags, StatusCode};
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{sync::mpsc, time::timeout};

    const ECHOED_SECRET: &str = "server-echoed-password-fixture";
    #[derive(Clone, Default)]
    struct Counters {
        auth: Arc<AtomicUsize>,
        opens: Arc<AtomicUsize>,
    }
    struct Server(Counters);
    struct Peer {
        counters: Counters,
        channels: HashMap<ChannelId, Channel<server::Msg>>,
    }
    impl server::Server for Server {
        type Handler = Peer;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
            Peer {
                counters: self.0.clone(),
                channels: HashMap::new(),
            }
        }
    }
    impl server::Handler for Peer {
        type Error = anyhow::Error;
        async fn auth_password(&mut self, _: &str, _: &str) -> anyhow::Result<Auth> {
            self.counters.auth.fetch_add(1, Ordering::SeqCst);
            Ok(Auth::Accept)
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
            session: &mut server::Session,
        ) -> anyhow::Result<()> {
            session.channel_success(id)?;
            Ok(())
        }
        async fn shell_request(
            &mut self,
            id: ChannelId,
            session: &mut server::Session,
        ) -> anyhow::Result<()> {
            session.channel_success(id)?;
            Ok(())
        }
        async fn subsystem_request(
            &mut self,
            id: ChannelId,
            name: &str,
            session: &mut server::Session,
        ) -> anyhow::Result<()> {
            if name == "sftp" {
                session.channel_success(id)?;
                let channel = self.channels.remove(&id).unwrap();
                let counters = self.counters.clone();
                tokio::spawn(async move {
                    russh_sftp::server::run(channel.into_stream(), Files(counters)).await;
                });
            }
            Ok(())
        }
    }
    struct Files(Counters);
    impl russh_sftp::server::Handler for Files {
        type Error = russh_sftp::server::StatusReply;
        fn unimplemented(&self) -> Self::Error {
            StatusCode::Failure.with_message(ECHOED_SECRET)
        }
        async fn stat(&mut self, id: u32, _: String) -> Result<Attrs, Self::Error> {
            Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o100644),
                    size: Some(1),
                    ..Default::default()
                },
            })
        }
        async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
            self.stat(id, path).await
        }
        async fn open(
            &mut self,
            _: u32,
            _: String,
            _: OpenFlags,
            _: FileAttributes,
        ) -> Result<Handle, Self::Error> {
            self.0.opens.fetch_add(1, Ordering::SeqCst);
            Err(self.unimplemented())
        }
    }
    struct Fixture {
        port: u16,
        fingerprint: String,
        counters: Counters,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Fixture {
        async fn new() -> Self {
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
                ..Default::default()
            });
            let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = socket.local_addr().unwrap().port();
            let counters = Counters::default();
            let mut server = Server(counters.clone());
            let task = tokio::spawn(async move {
                server.run_on_socket(config, &socket).await.unwrap();
            });
            Self {
                port,
                fingerprint,
                counters,
                task,
            }
        }
        fn profile(&self) -> Profile {
            let mut p = profile();
            p.host = "127.0.0.1".into();
            p.port = self.port;
            p
        }
        async fn connect(&self) -> (ssh::Session, mpsc::Receiver<ssh::Event>) {
            let (tx, mut rx) = mpsc::channel(32);
            let session = ssh::spawn(
                &tokio::runtime::Handle::current(),
                "test".into(),
                self.profile(),
                ECHOED_SECRET.into(),
                Some(self.fingerprint.clone()),
                tx,
            );
            loop {
                if matches!(
                    next(&mut rx).await,
                    ssh::Event::Status {
                        connected: true,
                        ..
                    }
                ) {
                    break;
                }
            }
            (session, rx)
        }
    }
    async fn next(rx: &mut mpsc::Receiver<ssh::Event>) -> ssh::Event {
        timeout(Duration::from_secs(15), rx.recv())
            .await
            .expect("event timeout")
            .expect("event queue closed")
    }
    async fn finish(session: ssh::Session) {
        let _ = session.commands.send(ssh::Command::Disconnect).await;
        timeout(Duration::from_secs(10), session.task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sftp_errors_never_echo_server_credentials_and_remove_partial_downloads() {
        let fixture = Fixture::new().await;
        let (session, mut rx) = fixture.connect().await;
        let temp = tempfile::tempdir().unwrap();
        session
            .commands
            .send(ssh::Command::DownloadFile {
                local_root: temp.path().to_path_buf(),
                transfer: 1,
                remote: "/file".into(),
                local: temp.path().join("file"),
            })
            .await
            .unwrap();
        let message = loop {
            if let ssh::Event::TransferFinished {
                error: true,
                message,
                ..
            } = next(&mut rx).await
            {
                break message;
            }
        };
        finish(session).await;
        assert_eq!(fixture.counters.opens.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(
            !message.contains(ECHOED_SECRET),
            "SFTP error exposed server text"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn legacy_download_rejects_unsafe_names_before_remote_open() {
        let fixture = Fixture::new().await;
        let (session, mut rx) = fixture.connect().await;
        let temp = tempfile::tempdir().unwrap();
        session
            .commands
            .send(ssh::Command::Download {
                local_root: temp.path().to_path_buf(),
                remote: "/file".into(),
                local: temp.path().join("file:stream"),
            })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut rx).await,
                ssh::Event::Transfer { error: true, .. }
            ) {
                break;
            }
        }
        finish(session).await;
        assert_eq!(fixture.counters.opens.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn uploads_reject_junction_ancestors_before_opening_remote_files() {
        let fixture = Fixture::new().await;
        let (session, mut rx) = fixture.connect().await;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("file"), "outside-private-key").unwrap();
        let link = temp.path().join("link");
        junction(&link, outside.path());
        for transfer in [0, 1] {
            let command = if transfer == 0 {
                ssh::Command::Upload {
                    local_root: temp.path().to_path_buf(),
                    directory: "/".into(),
                    local: link.join("file"),
                    overwrite: true,
                }
            } else {
                ssh::Command::UploadFile {
                    local_root: temp.path().to_path_buf(),
                    transfer,
                    directory: "/".into(),
                    local: link.join("file"),
                    overwrite: true,
                }
            };
            session.commands.send(command).await.unwrap();
            loop {
                if matches!(
                    next(&mut rx).await,
                    ssh::Event::Transfer { error: true, .. }
                        | ssh::Event::TransferFinished { error: true, .. }
                ) {
                    break;
                }
            }
        }
        finish(session).await;
        fs::remove_dir(&link).unwrap();
        assert_eq!(fixture.counters.opens.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn untrusted_keys_block_terminal_sftp_and_forwarding_before_authentication() {
        let fixture = Fixture::new().await;
        for known in [None, Some("SHA256:changed".into())] {
            let (tx, mut rx) = mpsc::channel(32);
            let session = ssh::spawn(
                &tokio::runtime::Handle::current(),
                "test".into(),
                fixture.profile(),
                ECHOED_SECRET.into(),
                known,
                tx,
            );
            session
                .commands
                .send(ssh::Command::List("/".into()))
                .await
                .unwrap();
            loop {
                match next(&mut rx).await {
                    ssh::Event::HostKey { reply, .. } => {
                        assert_eq!(fixture.counters.auth.load(Ordering::SeqCst), 0);
                        reply.send(false).unwrap();
                    }
                    ssh::Event::Closed { .. } => break,
                    ssh::Event::Status {
                        connected: true, ..
                    } => panic!("untrusted connection"),
                    _ => {}
                }
            }
            session.task.await.unwrap();
        }
        let host = fixture.profile();
        let rule = Forward {
            label: "Test".into(),
            host_id: host.id.clone(),
            bind_port: 9999,
            destination_host: "127.0.0.1".into(),
            destination_port: 22,
            ..Default::default()
        };
        assert!(
            forwarding::RunningRule::start(
                &tokio::runtime::Handle::current(),
                rule.clone(),
                host.clone(),
                zeroize::Zeroizing::new(ECHOED_SECRET.into()),
                None,
                String::new()
            )
            .is_err()
        );
        let mut running = forwarding::RunningRule::start(
            &tokio::runtime::Handle::current(),
            rule,
            host,
            zeroize::Zeroizing::new(ECHOED_SECRET.into()),
            None,
            "SHA256:changed".into(),
        )
        .unwrap();
        timeout(Duration::from_secs(15), async {
            while running.status().phase != forwarding::Phase::Error {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        running.stop().await;
        assert_eq!(fixture.counters.auth.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.counters.opens.load(Ordering::SeqCst), 0);
    }
}
