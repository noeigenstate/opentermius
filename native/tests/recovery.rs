use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use opentermius::{
    forwarding::Forward,
    keychain::KeyEntry,
    recovery::{self, RestoreMode},
    snippets::Snippet,
    store::{self, Data, Profile, Store},
};
use serde_json::json;
use std::fs;

fn data() -> Data {
    Data {
        version: 2,
        hosts_view: opentermius::hosts::HostsView {
            mode: "list".into(),
            sort: "last".into(),
        },
        hosts: vec![Profile {
            id: "host-1".into(),
            name: "Development".into(),
            host: "example.invalid".into(),
            port: 2222,
            username: "alice".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            group: "Work".into(),
            encrypted_secret: None,
            tags: vec!["work".into()],
            last_connected: Some("2026-01-02T03:04:05Z".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
        }],
        keys: vec![KeyEntry {
            id: "key-1".into(),
            ..Default::default()
        }],
        snippets: vec![Snippet {
            id: "snippet-1".into(),
            ..Default::default()
        }],
        forwards: vec![Forward {
            id: "forward-1".into(),
            ..Default::default()
        }],
        known_hosts: [(
            store::key("example.invalid", 2222),
            "SHA256:original".into(),
        )]
        .into(),
        ..Data::default()
    }
}
fn rewrite(code: &str, change: impl FnOnce(&mut Vec<u8>)) -> String {
    let mut bytes = URL_SAFE_NO_PAD
        .decode(code.strip_prefix("OTR1-").unwrap())
        .unwrap();
    change(&mut bytes);
    format!("OTR1-{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[test]
fn both_modes_roundtrip_every_section_and_persist() {
    for passphrase in [None, Some("correct horse recovery phrase")] {
        let source = data();
        let code = recovery::generate(&source, passphrase, true).unwrap();
        assert!(code.starts_with("OTR1-"));
        assert!(!code.contains("example.invalid"));
        let temp = tempfile::tempdir().unwrap();
        let mut destination = Store::open(temp.path().join("config.json")).unwrap();
        let result =
            recovery::restore(&mut destination, &code, passphrase, RestoreMode::Merge).unwrap();
        assert_eq!(
            (
                result.hosts,
                result.keys,
                result.snippets,
                result.forwards,
                result.known_hosts
            ),
            (1, 1, 1, 1, 1)
        );
        let reopened = Store::open(destination.path.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&reopened.data).unwrap(),
            serde_json::to_value(source).unwrap()
        );
    }
}

#[test]
fn restore_merges_or_replaces_hosts_view_preferences() {
    let source = data();
    let code = recovery::generate(&source, None, false).unwrap();
    for populated in [false, true] {
        for mode in [RestoreMode::Merge, RestoreMode::Replace] {
            let temp = tempfile::tempdir().unwrap();
            let mut destination = Store::open(temp.path().join("config.json")).unwrap();
            if populated {
                destination.data.hosts = source.hosts.clone();
            }
            let local_view = opentermius::hosts::HostsView {
                mode: "grid".into(),
                sort: "created".into(),
            };
            destination.data.hosts_view = local_view.clone();
            recovery::restore(&mut destination, &code, None, mode).unwrap();
            let expected = if mode == RestoreMode::Replace {
                &source.hosts_view
            } else {
                &local_view
            };
            assert_eq!(&destination.data.hosts_view, expected);
            let reopened = Store::open(destination.path.clone()).unwrap();
            assert_eq!(&reopened.data.hosts_view, expected);
        }
    }
}

#[test]
fn wrong_or_missing_passphrase_fails_without_changing_store() {
    let code = recovery::generate(&data(), Some("right"), true).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    for passphrase in [None, Some("wrong")] {
        assert!(
            recovery::restore(&mut destination, &code, passphrase, RestoreMode::Merge).is_err()
        );
        assert!(destination.data.hosts.is_empty());
        assert!(!destination.path.exists());
    }
}

#[test]
fn ciphertext_key_nonce_and_mode_tampering_fail() {
    let code = recovery::generate(&data(), None, true).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    for offset in [1, 3, 35, 48] {
        let changed = rewrite(&code, |bytes| bytes[offset] ^= 1);
        assert!(
            recovery::restore(
                &mut destination,
                &changed,
                Some("irrelevant"),
                RestoreMode::Merge
            )
            .is_err()
        );
        assert!(!destination.path.exists());
    }
}

#[test]
fn truncated_and_invalid_input_fail_without_panicking() {
    let code = recovery::generate(&data(), None, true).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    for value in [
        "",
        "OTR1-",
        "OTR1-!!!!",
        "OTR2-AAAA",
        &code[..8],
        &code[..code.len() - 8],
    ] {
        assert!(recovery::restore(&mut destination, value, None, RestoreMode::Merge).is_err());
    }
}

#[test]
fn inserted_unicode_whitespace_is_ignored() {
    let code = recovery::generate(&data(), None, true).unwrap();
    let spaced = code
        .chars()
        .enumerate()
        .fold(String::from(" \n\t"), |mut value, (i, c)| {
            value.push(c);
            if i % 19 == 0 {
                value.push_str("\r\n \t\u{2003}");
            }
            value
        });
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    assert_eq!(
        recovery::restore(&mut destination, &spaced, None, RestoreMode::Merge)
            .unwrap()
            .hosts,
        1
    );
}

#[test]
fn future_version_is_rejected_explicitly() {
    let code = recovery::generate(&data(), None, true).unwrap();
    let future = rewrite(&code, |bytes| bytes[0] = 2);
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    let error = recovery::restore(&mut destination, &future, None, RestoreMode::Merge).unwrap_err();
    assert!(error.to_string().contains("版本"));
    let mut source = data();
    source.version = 3;
    assert!(recovery::generate(&source, None, true).is_err());
}

#[test]
fn generic_walker_handles_nested_arrays_nulls_and_future_secret_names() {
    let original = json!({"outer": [{"encryptedFutureSecret": "secret", "publicKey": "public"},
        {"nested": {"encryptedPassphrase": "phrase", "encryptedOptional": null, "encryptedEmpty": ""}}],
        "encryptedContainer": {"encryptedPrivateKey": "private"}});
    let mut value = original.clone();
    recovery::transform_encrypted_fields(&mut value, &mut |s| Ok(format!("wrapped:{s}"))).unwrap();
    assert_eq!(value["outer"][0]["encryptedFutureSecret"], "wrapped:secret");
    assert_eq!(value["outer"][0]["publicKey"], "public");
    assert_eq!(value["outer"][1]["nested"]["encryptedEmpty"], "wrapped:");
    assert_eq!(
        value["outer"][1]["nested"]["encryptedPassphrase"],
        "wrapped:phrase"
    );
    assert_eq!(
        value["encryptedContainer"]["encryptedPrivateKey"],
        "wrapped:private"
    );
    recovery::transform_encrypted_fields(&mut value, &mut |s| {
        Ok(s.strip_prefix("wrapped:").unwrap().into())
    })
    .unwrap();
    assert_eq!(value, original);
}

#[cfg(windows)]
#[test]
fn keychain_private_key_and_host_reference_survive_recovery() {
    let entry =
        opentermius::keychain::generate("recovery key", opentermius::keychain::KeyKind::Ed25519)
            .unwrap();
    let original_pem =
        zeroize::Zeroizing::new(store::unprotect(&entry.encrypted_private_key).unwrap());
    let serialized = serde_json::to_value(&entry).unwrap();
    assert_eq!(
        serialized["encryptedPrivateKey"],
        entry.encrypted_private_key
    );
    let mut source = data();
    source.keys = vec![entry.clone()];
    source.hosts[0].auth_type = "key".into();
    source.hosts[0].key_id = entry.id.clone();
    assert!(source.hosts[0].private_key_path.is_empty());
    for passphrase in [None, Some("recovery passphrase")] {
        let code = recovery::generate(&source, passphrase, true).unwrap();
        for mode in [RestoreMode::Merge, RestoreMode::Replace] {
            let temp = tempfile::tempdir().unwrap();
            let mut destination = Store::open(temp.path().join("config.json")).unwrap();
            let summary = recovery::restore(&mut destination, &code, passphrase, mode).unwrap();
            assert_eq!((summary.hosts, summary.keys, summary.key_files), (1, 1, 0));
            let mut reopened = Store::open(destination.path.clone()).unwrap();
            let restored = &mut reopened.data.keys[0];
            restored.validate().unwrap();
            assert_eq!(restored.id, entry.id);
            assert_eq!(restored.public_key, entry.public_key);
            assert_eq!(restored.fingerprint, entry.fingerprint);
            assert_ne!(restored.encrypted_private_key, entry.encrypted_private_key);
            assert_eq!(
                store::unprotect(&restored.encrypted_private_key).unwrap(),
                *original_pem
            );
            let mut host = reopened.data.hosts[0].clone();
            host.validate().unwrap();
            assert_eq!(host.key_id, entry.id);
            assert!(host.private_key_path.is_empty());
            let key = reopened
                .key_material(&host.key_id)
                .unwrap()
                .into_private_key()
                .unwrap();
            assert_eq!(key.public_key().to_openssh().unwrap(), entry.public_key);
            // Saving also validates that the restored key_id resolves to an entry.
            reopened.save(host, None, false).unwrap();
            assert!(
                !fs::read_to_string(&reopened.path)
                    .unwrap()
                    .contains(original_pem.as_str())
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn dpapi_credentials_are_portable_and_reencrypted() {
    let mut source = data();
    source.hosts[0].encrypted_secret = Some(store::protect("saved-test-password").unwrap());
    let code = recovery::generate(&source, None, true).unwrap();
    assert!(!code.contains("saved-test-password"));
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    recovery::restore(&mut destination, &code, None, RestoreMode::Merge).unwrap();
    let encrypted = destination.data.hosts[0].encrypted_secret.as_ref().unwrap();
    assert_eq!(store::unprotect(encrypted).unwrap(), "saved-test-password");
    assert_ne!(
        encrypted,
        source.hosts[0].encrypted_secret.as_ref().unwrap()
    );
    assert!(
        !fs::read_to_string(&destination.path)
            .unwrap()
            .contains("saved-test-password")
    );
    let mut nested = json!({"keys": [{"encryptedPrivateKey": store::protect("private key").unwrap(),
        "encryptedPassphrase": store::protect("passphrase").unwrap()}]});
    recovery::transform_encrypted_fields(&mut nested, &mut store::unprotect).unwrap();
    assert_eq!(nested["keys"][0]["encryptedPrivateKey"], "private key");
    recovery::transform_encrypted_fields(&mut nested, &mut store::protect).unwrap();
    assert_eq!(
        store::unprotect(nested["keys"][0]["encryptedPassphrase"].as_str().unwrap()).unwrap(),
        "passphrase"
    );
}

#[cfg(windows)]
#[test]
fn protected_empty_credentials_are_distinct_from_absent_credentials() {
    let mut source = data();
    source.hosts[0].encrypted_secret = Some(store::protect("").unwrap());
    let code = recovery::generate(&source, None, true).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    recovery::restore(&mut destination, &code, None, RestoreMode::Merge).unwrap();
    let secret = destination.data.hosts[0].encrypted_secret.as_ref().unwrap();
    assert!(!secret.is_empty());
    assert_eq!(store::unprotect(secret).unwrap(), "");
    let code = recovery::generate(&source, None, false).unwrap();
    recovery::restore(&mut destination, &code, None, RestoreMode::Replace).unwrap();
    assert!(destination.data.hosts[0].encrypted_secret.is_none());
}

#[test]
fn excluding_secrets_does_not_try_to_decrypt_or_read_key_files() {
    let mut source = data();
    source.hosts[0].encrypted_secret = Some("unavailable-on-this-machine".into());
    source.hosts[0].private_key_path = "missing-private-key".into();
    assert!(recovery::generate(&source, None, true).is_err());
    let code = recovery::generate(&source, None, false).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    let result = recovery::restore(&mut destination, &code, None, RestoreMode::Merge).unwrap();
    assert!(destination.data.hosts[0].encrypted_secret.is_none());
    assert_eq!(result.key_files, 0);
}

#[test]
fn key_files_are_embedded_and_restored_without_overwriting_unrelated_files() {
    let source_dir = tempfile::tempdir().unwrap();
    let original = source_dir.path().join("id_test");
    fs::write(&original, b"test-only-private-key\0binary").unwrap();
    let mut source = data();
    source.hosts[0].auth_type = "key".into();
    source.hosts[0].private_key_path = original.to_string_lossy().into();
    let code = recovery::generate(&source, None, true).unwrap();
    fs::remove_file(&original).unwrap();
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("keys")).unwrap();
    let unrelated = temp.path().join("keys/host-1.key");
    fs::write(&unrelated, "unrelated").unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    let result = recovery::restore(&mut destination, &code, None, RestoreMode::Merge).unwrap();
    assert_eq!(result.key_files, 1);
    let restored = std::path::Path::new(&destination.data.hosts[0].private_key_path);
    assert_eq!(restored.parent(), Some(temp.path().join("keys").as_path()));
    assert_ne!(restored, unrelated);
    assert_eq!(
        fs::read(restored).unwrap(),
        b"test-only-private-key\0binary"
    );
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "unrelated");
    let result = recovery::restore(&mut destination, &code, None, RestoreMode::Merge).unwrap();
    assert_eq!(result.key_files, 0);
}

#[test]
fn merge_keeps_existing_replace_updates_matching_ids_and_preserves_pins() {
    let source = data();
    let code = recovery::generate(&source, None, true).unwrap();
    for mode in [RestoreMode::Merge, RestoreMode::Replace] {
        let temp = tempfile::tempdir().unwrap();
        let mut destination = Store::open(temp.path().join("config.json")).unwrap();
        destination.data = source.clone();
        destination.data.hosts[0].name = "Local".into();
        let mut local_only = destination.data.hosts[0].clone();
        local_only.id = "host-local".into();
        destination.data.hosts.push(local_only);
        let pin = store::key("example.invalid", 2222);
        destination
            .data
            .known_hosts
            .insert(pin.clone(), "SHA256:local".into());
        let result = recovery::restore(&mut destination, &code, None, mode).unwrap();
        assert_eq!(destination.data.hosts.len(), 2);
        assert_eq!(
            destination.data.hosts[0].name,
            if mode == RestoreMode::Merge {
                "Local"
            } else {
                "Development"
            }
        );
        assert_eq!(result.fingerprint_conflicts, vec![pin.clone()]);
        assert_eq!(destination.data.known_hosts[&pin], "SHA256:local");
        if mode == RestoreMode::Merge {
            assert_eq!((result.hosts, result.skipped), (0, 4));
        } else {
            assert_eq!(
                (result.hosts, result.keys, result.snippets, result.forwards),
                (1, 1, 1, 1)
            );
        }
    }
}

#[test]
fn persistence_failure_rolls_back_data_and_new_key_files() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.key");
    fs::write(&original, "test-key").unwrap();
    let mut source = data();
    source.hosts[0].private_key_path = original.to_string_lossy().into();
    let code = recovery::generate(&source, None, true).unwrap();
    let path = temp.path().join("config-is-a-directory");
    fs::create_dir(&path).unwrap();
    let mut destination = Store {
        path,
        data: Data {
            version: 2,
            ..Default::default()
        },
    };
    assert!(recovery::restore(&mut destination, &code, None, RestoreMode::Replace).is_err());
    assert!(destination.data.hosts.is_empty());
    assert_eq!(fs::read_dir(temp.path().join("keys")).unwrap().count(), 0);
}

#[test]
fn malformed_profiles_and_path_traversal_ids_are_rejected() {
    let mut source = data();
    source.hosts[0].port = 0;
    let code = recovery::generate(&source, None, true).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut destination = Store::open(temp.path().join("config.json")).unwrap();
    assert!(recovery::restore(&mut destination, &code, None, RestoreMode::Merge).is_err());
    source.hosts[0].id = "../escape".into();
    assert!(recovery::generate(&source, None, true).is_err());
}

#[test]
fn aggregate_key_file_size_is_bounded_before_collecting_the_entire_backup() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large-test.key");
    fs::write(&path, vec![b'x'; 1024 * 1024]).unwrap();
    let mut source = data();
    let mut host = source.hosts.pop().unwrap();
    host.private_key_path = path.to_string_lossy().into();
    for i in 0..20 {
        host.id = format!("host-{i}");
        source.hosts.push(host.clone());
    }
    assert!(
        recovery::generate(&source, None, true)
            .unwrap_err()
            .to_string()
            .contains("总量")
    );
}
