//! Cross-feature workflows using real persistence, DPAPI and loopback SSH only.
use opentermius::{
    forwarding::Forward,
    import_xshell,
    keychain::{self, KeyKind},
    recovery::{self, RestoreMode},
    settings::{self, Appearance, ColorScheme, Settings},
    snippets::{self, Snippet},
    store::{self, Profile, Store},
};
use serde_json::{Value, json};
use std::{collections::HashSet, fs};

fn host(id: &str) -> Profile {
    serde_json::from_value(json!({
        "id": id, "name": "Integration host", "host": "127.0.0.1",
        "username": "tester", "group": "Work/QA", "tags": ["qa", "ssh"]
    }))
    .unwrap()
}

fn snapshot(store: &Store) -> Value {
    serde_json::to_value(&store.data).unwrap()
}

fn preferences() -> Settings {
    Settings {
        font_size: 21,
        scrollback_lines: 4321,
        cursor_blink: false,
        copy_on_select: true,
        confirm_multiline_paste: false,
        color_scheme: ColorScheme::Monokai,
        appearance: Appearance::Dark,
        keepalive_seconds: 0,
        connection_timeout_seconds: 27,
        confirm_overwrite: false,
        show_hidden_files: false,
        ..Default::default()
    }
}

#[test]
fn legacy_defaults_then_current_config_is_a_golden_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(&path, br#"{"version":2,"hosts":[]}"#).unwrap();
    let mut store = Store::open(path.clone()).unwrap();
    assert_eq!(store.data.settings, Settings::default());
    assert!(store.data.keys.is_empty());
    assert!(store.data.forwards.is_empty());
    assert!(store.data.snippets.is_empty());
    assert!(store.data.known_hosts.is_empty());
    settings::persist(&mut store, preferences()).unwrap();
    store.save(host("golden-host"), None, false).unwrap();
    snippets::save(
        &mut store.data.snippets,
        Snippet {
            name: "Health".into(),
            script: "printf healthy\n".into(),
            ..Default::default()
        },
        1_700_000_000,
    )
    .unwrap();
    store.persist().unwrap();
    let expected = snapshot(&store);
    let bytes = fs::read(&path).unwrap();
    let reopened = Store::open(path).unwrap();
    assert_eq!(snapshot(&reopened), expected);
    reopened.persist().unwrap();
    assert_eq!(fs::read(&reopened.path).unwrap(), bytes);
}

#[test]
fn xshell_import_groups_and_dedup_survive_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let sessions = temp.path().join("Sessions");
    fs::create_dir_all(sessions.join("Work/QA")).unwrap();
    let session = "[CONNECTION]\nProtocol=SSH\nHost=qa.example.invalid\nPort=2222\n[CONNECTION:AUTHENTICATION]\nUserName=tester\nPassword=vendor-secret-must-not-import\n";
    fs::write(sessions.join("Work/QA/server.xsh"), session).unwrap();
    fs::write(sessions.join("duplicate.XSH"), session).unwrap();
    let mut source = Store::open(temp.path().join("source.json")).unwrap();
    let summary = import_xshell::merge(&mut source, import_xshell::scan(&sessions)).unwrap();
    assert_eq!((summary.imported, summary.duplicates), (1, 1));
    assert_eq!(source.data.hosts[0].group, "Work/QA");
    assert!(source.data.hosts[0].encrypted_secret.is_none());
    let code = recovery::generate(&source.data, None, true).unwrap();
    let mut restored = Store::open(temp.path().join("restored/config.json")).unwrap();
    recovery::restore(&mut restored, &code, None, RestoreMode::Merge).unwrap();
    let summary = import_xshell::merge(&mut restored, import_xshell::scan(&sessions)).unwrap();
    assert_eq!((summary.imported, summary.duplicates), (0, 2));
    assert_eq!(
        snapshot(&Store::open(restored.path.clone()).unwrap()),
        snapshot(&source)
    );
    assert!(
        !fs::read_to_string(restored.path)
            .unwrap()
            .contains("vendor-secret")
    );
}

#[cfg(windows)]
#[test]
fn duplicate_key_ids_are_rejected_without_breaking_host_or_backup() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    let first = keychain::generate("first", KeyKind::Ed25519).unwrap();
    let mut second = keychain::generate("second", KeyKind::Ed25519).unwrap();
    second.id = first.id.clone();
    store.add_key(first.clone()).unwrap();
    let mut profile = host("key-host");
    profile.auth_type = "key".into();
    profile.key_id = first.id.clone();
    store.save(profile, None, false).unwrap();
    let before = snapshot(&store);
    let disk = fs::read(&store.path).unwrap();
    assert!(
        store.add_key(second).is_err(),
        "duplicate key IDs must be refused"
    );
    assert_eq!(snapshot(&store), before);
    assert_eq!(fs::read(&store.path).unwrap(), disk);
    assert_eq!(
        store
            .key_material(&first.id)
            .unwrap()
            .into_private_key()
            .unwrap()
            .public_key()
            .to_openssh()
            .unwrap(),
        first.public_key
    );
    assert!(recovery::generate(&store.data, None, true).is_ok());
}

#[cfg(windows)]
#[test]
fn populated_recovery_restores_secrets_files_and_preferences_in_both_modes() {
    let source_dir = tempfile::tempdir().unwrap();
    let mut source = Store::open(source_dir.path().join("config.json")).unwrap();
    let entry = keychain::generate("portable", KeyKind::Ed25519).unwrap();
    let pem = entry.export_private().unwrap();
    source.add_key(entry.clone()).unwrap();
    source
        .save(
            host("password-host"),
            Some("integration-only-password"),
            true,
        )
        .unwrap();
    let mut keyed = host("keychain-host");
    keyed.auth_type = "key".into();
    keyed.key_id = entry.id.clone();
    source.save(keyed, None, false).unwrap();
    let key_file = source_dir.path().join("original.key");
    fs::write(&key_file, pem.as_bytes()).unwrap();
    let mut file_host = host("file-host");
    file_host.auth_type = "key".into();
    file_host.private_key_path = key_file.to_string_lossy().into_owned();
    source
        .save(file_host, Some("file-key-passphrase"), true)
        .unwrap();
    snippets::save(
        &mut source.data.snippets,
        Snippet {
            name: "Deploy".into(),
            script: "echo ready\necho done\n".into(),
            tags: vec!["qa".into()],
            ..Default::default()
        },
        1_700_000_000,
    )
    .unwrap();
    let mut forward = Forward {
        label: "QA tunnel".into(),
        host_id: "keychain-host".into(),
        bind_port: 41234,
        destination_host: "127.0.0.1".into(),
        destination_port: 8080,
        ..Default::default()
    };
    forward.validate(&[]).unwrap();
    source.data.forwards.push(forward);
    source.data.hosts_view.mode = "list".into();
    source.data.hosts_view.sort = "last".into();
    let known = store::parse_known_hosts(&format!("127.0.0.1 {}", entry.public_key));
    assert_eq!(known.skipped, 0);
    assert_eq!(source.import_known_hosts(&known.entries).unwrap().added, 1);
    settings::persist(&mut source, preferences()).unwrap();
    let code = recovery::generate(&source.data, Some("portable phrase"), true).unwrap();
    fs::remove_file(&key_file).unwrap();
    for mode in [RestoreMode::Merge, RestoreMode::Replace] {
        let temp = tempfile::tempdir().unwrap();
        let mut target = Store::open(temp.path().join("config.json")).unwrap();
        target.persist().unwrap(); // A saved empty installation is still fresh.
        let summary = recovery::restore(&mut target, &code, Some("portable phrase"), mode).unwrap();
        assert_eq!(
            (
                summary.hosts,
                summary.keys,
                summary.snippets,
                summary.forwards,
                summary.known_hosts,
                summary.key_files
            ),
            (3, 1, 1, 1, 1, 1)
        );
        let restored = Store::open(target.path.clone()).unwrap();
        assert_eq!(snapshot(&restored), snapshot(&target));
        assert_eq!(restored.data.settings, preferences());
        assert_eq!(restored.data.hosts_view, source.data.hosts_view);
        assert_eq!(restored.data.known_hosts, source.data.known_hosts);
        assert_eq!(
            serde_json::to_value(&restored.data.snippets).unwrap(),
            serde_json::to_value(&source.data.snippets).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&restored.data.forwards).unwrap(),
            serde_json::to_value(&source.data.forwards).unwrap()
        );
        let password = restored
            .data
            .hosts
            .iter()
            .find(|h| h.id == "password-host")
            .unwrap();
        assert_eq!(
            store::unprotect(password.encrypted_secret.as_deref().unwrap()).unwrap(),
            "integration-only-password"
        );
        let file_host = restored
            .data
            .hosts
            .iter()
            .find(|h| h.id == "file-host")
            .unwrap();
        assert_eq!(
            fs::read(&file_host.private_key_path).unwrap(),
            pem.as_bytes()
        );
        assert!(
            std::path::Path::new(&file_host.private_key_path).starts_with(temp.path().join("keys"))
        );
        assert_eq!(
            store::unprotect(file_host.encrypted_secret.as_deref().unwrap()).unwrap(),
            "file-key-passphrase"
        );
        assert_eq!(
            restored
                .key_material(&entry.id)
                .unwrap()
                .into_private_key()
                .unwrap()
                .public_key()
                .to_openssh()
                .unwrap(),
            entry.public_key
        );
        let ids: Vec<_> = restored
            .data
            .hosts
            .iter()
            .map(|h| &h.id)
            .chain(restored.data.keys.iter().map(|k| &k.id))
            .chain(restored.data.snippets.iter().map(|s| &s.id))
            .chain(restored.data.forwards.iter().map(|f| &f.id))
            .collect();
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
        let disk = fs::read_to_string(&restored.path).unwrap();
        assert!(!disk.contains("integration-only-password"));
        assert!(!disk.contains("file-key-passphrase"));
        assert!(!disk.contains("BEGIN OPENSSH PRIVATE KEY"));
        let expected = snapshot(&target);
        let again = recovery::restore(
            &mut target,
            &code,
            Some("portable phrase"),
            RestoreMode::Merge,
        )
        .unwrap();
        assert_eq!(
            (
                again.hosts,
                again.keys,
                again.snippets,
                again.forwards,
                again.key_files,
                again.skipped
            ),
            (0, 0, 0, 0, 0, 6)
        );
        assert_eq!(snapshot(&target), expected);
    }
}

#[cfg(windows)]
#[test]
fn host_deletion_releases_key_but_preserves_detectable_orphan_forward() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    let entry = keychain::generate("in use", KeyKind::Ed25519).unwrap();
    store.add_key(entry.clone()).unwrap();
    let mut profile = host("referenced");
    profile.auth_type = "key".into();
    profile.key_id = entry.id.clone();
    store.save(profile, None, false).unwrap();
    store.data.forwards.push(Forward {
        id: "orphan-rule".into(),
        host_id: "referenced".into(),
        ..Default::default()
    });
    store.persist().unwrap();
    let before = snapshot(&store);
    assert!(
        store
            .delete_key(&entry.id)
            .unwrap_err()
            .to_string()
            .contains("is using this key")
    );
    assert_eq!(snapshot(&store), before);
    assert_eq!(snapshot(&Store::open(store.path.clone()).unwrap()), before);
    store.delete("referenced").unwrap();
    store.delete_key(&entry.id).unwrap();
    let reopened = Store::open(store.path.clone()).unwrap();
    assert_eq!(reopened.data.forwards[0].host_id, "referenced");
    assert!(
        !reopened
            .data
            .hosts
            .iter()
            .any(|h| h.id == reopened.data.forwards[0].host_id)
    );
    assert!(reopened.key_material(&entry.id).is_err());
    let code = recovery::generate(&reopened.data, None, false).unwrap();
    let mut target = Store::open(temp.path().join("restored.json")).unwrap();
    recovery::restore(&mut target, &code, None, RestoreMode::Merge).unwrap();
    assert_eq!(snapshot(&target), snapshot(&reopened));
}

#[test]
fn garbage_imports_and_recovery_are_bounded_and_do_not_mutate_config() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("config.json")).unwrap();
    store.save(host("unchanged"), None, false).unwrap();
    let before = snapshot(&store);
    let disk = fs::read(&store.path).unwrap();
    let sessions = temp.path().join("garbage");
    fs::create_dir(&sessions).unwrap();
    let mut state = 0x1234_5678u32;
    for index in 0..64 {
        let mut bytes = vec![0; index * 13 + 1];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        fs::write(sessions.join(format!("{index}.xsh")), &bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let code = format!("OTR1-{text}");
        assert!(recovery::restore(&mut store, &code, None, RestoreMode::Merge).is_err());
        assert!(store::parse_known_hosts(&text).entries.is_empty());
    }
    fs::write(sessions.join("huge.xsh"), vec![b'x'; 5 * 1024 * 1024 + 1]).unwrap();
    let scanned = import_xshell::scan(&sessions);
    assert!(scanned.hosts.is_empty());
    assert_eq!(scanned.skipped.len(), 65);
    assert_eq!(
        import_xshell::merge(&mut store, scanned).unwrap().imported,
        0
    );
    let huge = format!("OTR1-{}", "A".repeat(20 * 1024 * 1024));
    assert!(
        recovery::restore(&mut store, &huge, None, RestoreMode::Replace)
            .unwrap_err()
            .to_string()
            .contains("too long")
    );
    assert!(
        store::parse_known_hosts(&"x".repeat(1024 * 1024))
            .entries
            .is_empty()
    );
    assert_eq!(snapshot(&store), before);
    assert_eq!(fs::read(&store.path).unwrap(), disk);
}
