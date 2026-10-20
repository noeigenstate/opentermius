#[cfg(windows)]
use base64::{Engine, engine::general_purpose::STANDARD};
use opentermius::store::{self, Data, Profile, Store};
use std::fs;
use tempfile::tempdir;

fn profile() -> Profile {
    Profile {
        id: String::new(),
        name: "Test host".into(),
        host: "example.invalid".into(),
        port: 22,
        username: "tester".into(),
        auth_type: "password".into(),
        private_key_path: String::new(),
        group: String::new(),
        encrypted_secret: None,
    }
}

fn store_in(dir: &tempfile::TempDir) -> Store {
    Store::open(dir.path().join("opentermius-native.json")).unwrap()
}

fn write_json(path: &std::path::Path, value: &str) {
    fs::write(path, value).unwrap();
}

#[test]
fn validate_requires_host_and_defaults_the_display_name() {
    let mut p = profile();
    p.host = "   ".into();
    assert!(p.validate().is_err());

    let mut p = profile();
    p.name = "   ".into();
    p.host = "  example.invalid  ".into();
    p.validate().unwrap();
    assert_eq!(p.host, "example.invalid");
    assert_eq!(p.name, "example.invalid");
    assert!(!p.id.is_empty());
}

#[test]
fn default_display_names_for_long_hosts_survive_revalidation_and_reopening() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    let mut p = profile();
    p.name.clear();
    p.host = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    p.validate().unwrap();
    assert_eq!(p.host.len(), 253);
    assert!(
        p.validate().is_ok(),
        "a validated profile must remain valid"
    );
    store.save(p, None, false).unwrap();
    let reopened = Store::open(store.path.clone()).unwrap();
    assert_eq!(reopened.data.hosts[0].host.len(), 253);
    assert!(reopened.data.hosts[0].name.chars().count() <= 128);
}

#[test]
fn validate_accepts_boundary_ports_and_rejects_zero() {
    for port in [1u16, 22, 65535] {
        let mut p = profile();
        p.port = port;
        p.validate().unwrap();
    }
    let mut p = profile();
    p.port = 0;
    assert!(p.validate().is_err());
}

#[test]
fn validate_accepts_only_known_auth_types_and_requires_a_key_path() {
    let mut p = profile();
    for auth in ["password", "key"] {
        p.auth_type = auth.into();
        if auth == "key" {
            p.private_key_path = "C:\\keys\\id".into();
        }
        p.validate().unwrap();
    }

    let mut p = profile();
    p.auth_type = "agent".into();
    assert!(p.validate().is_err());

    let mut p = profile();
    p.auth_type = "key".into();
    p.private_key_path = String::new();
    assert!(p.validate().is_err());

    let mut p = profile();
    p.auth_type = "key".into();
    p.private_key_path = "   ".into();
    assert!(
        p.validate().is_err(),
        "whitespace is not a private key path"
    );

    let mut p = profile();
    p.auth_type = "key".into();
    p.private_key_path = "  C:\\keys\\id  ".into();
    p.validate().unwrap();
    assert_eq!(p.private_key_path, "C:\\keys\\id");
}

#[test]
fn validate_limits_text_fields_by_characters_not_only_bytes() {
    let mut p = profile();
    p.name = "a".repeat(128);
    p.validate().unwrap();

    p.name = "a".repeat(129);
    assert!(
        p.validate().is_err(),
        "129 ASCII characters exceed the limit"
    );

    p.name = "你".repeat(128);
    p.validate().unwrap();
    p.name = "你".repeat(129);
    assert!(
        p.validate().is_err(),
        "129 wide characters exceed the limit"
    );

    p.name = "name".into();
    p.host = "a".repeat(253);
    p.validate().unwrap();
    p.host = "a".repeat(254);
    assert!(p.validate().is_err());

    p.host = "example.invalid".into();
    p.username = "u".repeat(129);
    assert!(p.validate().is_err());
    p.username = "u".repeat(128);
    p.group = "g".repeat(129);
    assert!(p.validate().is_err());
    p.group = "g".repeat(128);
    p.validate().unwrap();
}

#[test]
fn validate_rejects_control_characters_and_whitespace_in_hosts() {
    for host in ["bad host", "bad\thost", "bad\rhost", "bad\nhost"] {
        let mut p = profile();
        p.host = host.into();
        assert!(p.validate().is_err(), "host {host:?} must be rejected");
    }
    for field in 0..4 {
        let mut p = profile();
        match field {
            0 => p.name = "bad\nname".into(),
            1 => p.host = "bad\u{1}host".into(),
            2 => p.username = "bad\nuser".into(),
            _ => p.group = "bad\u{7}group".into(),
        }
        assert!(p.validate().is_err());
    }
}

#[test]
fn validate_rejects_oversized_or_controlled_private_key_paths() {
    let mut p = profile();
    p.auth_type = "key".into();
    p.private_key_path = format!("C:\\keys\\{}", "x".repeat(40_000));
    assert!(p.validate().is_err());

    let mut p = profile();
    p.auth_type = "key".into();
    p.private_key_path = "C:\\keys\\bad\nname".into();
    assert!(p.validate().is_err());
}

#[test]
fn validate_accepts_unicode_names_groups_and_users() {
    let mut p = profile();
    p.name = "服务器 🚀".into();
    p.host = "例子.测试".into();
    p.username = "用户".into();
    p.group = "生产环境".into();
    p.validate().unwrap();
    assert_eq!(p.name, "服务器 🚀");
}

#[test]
fn open_missing_file_returns_an_empty_unwritten_store() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("config.json");
    let store = Store::open(path.clone()).unwrap();
    assert_eq!(store.data.version, 2);
    assert!(store.data.hosts.is_empty());
    assert!(store.data.known_hosts.is_empty());
    assert!(store.data.keys.is_empty());
    assert!(store.data.forwards.is_empty());
    assert!(store.data.snippets.is_empty());
    assert!(!path.exists(), "opening must not create a config file");
}

#[test]
fn open_applies_serde_defaults_and_validates_stored_hosts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    write_json(
        &path,
        r#"{"version":2,"hosts":[{"name":"","host":" example.invalid ","username":"tester"}]}"#,
    );
    let store = Store::open(path).unwrap();
    let id = {
        let p = &store.data.hosts[0];
        assert_eq!(p.host, "example.invalid");
        assert_eq!(p.name, "example.invalid");
        assert_eq!(p.port, 22);
        assert_eq!(p.auth_type, "password");
        assert_eq!(p.group, "");
        assert_eq!(p.private_key_path, "");
        assert!(p.encrypted_secret.is_none());
        assert!(!p.id.is_empty());
        p.id.clone()
    };
    assert_eq!(store.data.version, 2);

    // A generated id is persisted by the next write.
    store.persist().unwrap();
    let reopened = Store::open(store.path.clone()).unwrap();
    assert_eq!(reopened.data.hosts[0].id, id);
}

#[test]
fn open_rejects_corrupt_oversized_and_wrong_version_files() {
    let dir = tempdir().unwrap();

    let corrupt = dir.path().join("corrupt.json");
    write_json(&corrupt, "{ not json");
    let error = Store::open(corrupt).err().unwrap().to_string();
    assert!(error.contains("损坏"), "unexpected error: {error}");

    let empty = dir.path().join("empty.json");
    write_json(&empty, "");
    assert!(Store::open(empty).is_err());

    let wrong = dir.path().join("wrong-version.json");
    write_json(&wrong, r#"{"version":1,"hosts":[]}"#);
    let error = Store::open(wrong).err().unwrap().to_string();
    assert!(error.contains("版本"), "unexpected error: {error}");

    let missing_version = dir.path().join("missing-version.json");
    write_json(&missing_version, r#"{"hosts":[]}"#);
    assert!(Store::open(missing_version).is_err());

    let invalid_host = dir.path().join("invalid-host.json");
    write_json(
        &invalid_host,
        r#"{"version":2,"hosts":[{"name":"x","host":"example.invalid","username":"u","port":0}]}"#,
    );
    assert!(Store::open(invalid_host).is_err());

    let oversized = dir.path().join("oversized.json");
    let file = fs::File::create(&oversized).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    let error = Store::open(oversized).err().unwrap().to_string();
    assert!(error.contains("过大"), "unexpected error: {error}");
}

#[test]
fn open_accepts_utf8_configuration_and_trims_display_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    write_json(
        &path,
        r#"{"version":2,"hosts":[{"name":" 服务器 ","host":"例子.测试","username":"用户","authType":"password"}]}"#,
    );
    let store = Store::open(path).unwrap();
    assert_eq!(store.data.hosts[0].name, "服务器");
    assert_eq!(store.data.hosts[0].host, "例子.测试");
}

#[test]
fn persist_replaces_the_file_and_leaves_no_temporary_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut store = Store::open(path.clone()).unwrap();
    let mut p = profile();
    store.save(p.clone(), None, false).unwrap();
    p.id = store.data.hosts[0].id.clone();
    p.name = "Renamed".into();
    store.save(p, None, false).unwrap();

    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.hosts.len(), 1);
    assert_eq!(reopened.data.hosts[0].name, "Renamed");

    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["config.json"]);
}

#[test]
fn persist_failure_cleans_the_temp_file_and_rolls_back_hosts() {
    let dir = tempdir().unwrap();
    let blocked = dir.path().join("blocked");
    fs::create_dir(&blocked).unwrap();

    let mut p = profile();
    p.id = "existing".into();
    let mut store = Store {
        path: blocked.clone(),
        data: Data {
            version: 2,
            hosts: vec![p.clone()],
            ..Data::default()
        },
    };

    let mut saved = profile();
    saved.name = "Second".into();
    assert!(store.save(saved, None, false).is_err());
    assert_eq!(store.data.hosts.len(), 1);
    assert_eq!(store.data.hosts[0].id, "existing");

    assert!(store.delete("existing").is_err());
    assert_eq!(store.data.hosts.len(), 1);

    assert!(store.trust("example.invalid", 22, "SHA256:test").is_err());
    assert!(store.data.known_hosts.is_empty());

    let import = dir.path().join("import.json");
    write_json(
        &import,
        r#"{"version":2,"hosts":[{"name":"Imported","host":"import.invalid","username":"u"}]}"#,
    );
    assert!(store.import(&import).is_err());
    assert_eq!(store.data.hosts.len(), 1);

    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
}

#[test]
fn persist_creates_missing_parent_directories() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("one").join("two").join("config.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.save(profile(), None, false).unwrap();
    assert!(path.is_file());
}

#[test]
fn save_updates_by_id_and_rolls_back_invalid_profiles() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    let id = store.data.hosts[0].id.clone();

    let mut updated = profile();
    updated.id = id.clone();
    updated.host = "updated.invalid".into();
    updated.name = "Updated".into();
    store.save(updated, None, false).unwrap();
    assert_eq!(store.data.hosts.len(), 1);
    assert_eq!(store.data.hosts[0].host, "updated.invalid");

    let mut invalid = store.data.hosts[0].clone();
    invalid.port = 0;
    invalid.host = "never.invalid".into();
    assert!(store.save(invalid, None, false).is_err());
    assert_eq!(store.data.hosts.len(), 1);
    assert_eq!(store.data.hosts[0].host, "updated.invalid");
    assert_eq!(store.data.hosts[0].port, 22);
}

#[test]
fn save_rejects_an_invalid_profile_without_touching_the_file() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    let before = fs::read(&store.path).unwrap();

    let mut invalid = profile();
    invalid.host = "bad host".into();
    assert!(store.save(invalid, None, false).is_err());
    assert_eq!(fs::read(&store.path).unwrap(), before);
    assert_eq!(store.data.hosts.len(), 1);
}

#[cfg(windows)]
#[test]
fn save_encrypts_remembered_secrets_and_preserves_them_on_update() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), Some("first-secret"), true).unwrap();
    let id = store.data.hosts[0].id.clone();
    let first = store.data.hosts[0].encrypted_secret.clone().unwrap();
    assert!(!first.contains("first-secret"));
    assert_eq!(store::unprotect(&first).unwrap(), "first-secret");
    assert!(
        !fs::read_to_string(&store.path)
            .unwrap()
            .contains("first-secret")
    );

    let mut update = store.data.hosts[0].clone();
    update.name = "Updated".into();
    store.save(update, None, true).unwrap();
    assert_eq!(store.data.hosts[0].id, id);
    assert_eq!(
        store.data.hosts[0].encrypted_secret.as_deref(),
        Some(first.as_str())
    );

    let mut update = store.data.hosts[0].clone();
    update.name = "Again".into();
    store.save(update, Some("second-secret"), true).unwrap();
    let second = store.data.hosts[0].encrypted_secret.clone().unwrap();
    assert_ne!(first, second);
    assert_eq!(store::unprotect(&second).unwrap(), "second-secret");

    let mut update = store.data.hosts[0].clone();
    update.auth_type = "key".into();
    update.private_key_path = "C:\\keys\\id".into();
    store.save(update, None, true).unwrap();
    assert!(store.data.hosts[0].encrypted_secret.is_none());

    let update = store.data.hosts[0].clone();
    store.save(update, Some("key-passphrase"), true).unwrap();
    assert!(store.data.hosts[0].encrypted_secret.is_some());
    let update = store.data.hosts[0].clone();
    store.save(update, None, false).unwrap();
    assert!(store.data.hosts[0].encrypted_secret.is_none());
}

#[cfg(windows)]
#[test]
fn save_rejects_oversized_secrets_without_mutating_data() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    let id = store.data.hosts[0].id.clone();
    let before = fs::read(&store.path).unwrap();

    let mut update = store.data.hosts[0].clone();
    update.name = "Never".into();
    assert!(store.save(update, Some(&"x".repeat(16385)), true).is_err());
    assert_eq!(store.data.hosts[0].id, id);
    assert_eq!(store.data.hosts[0].name, "Test host");
    assert_eq!(fs::read(&store.path).unwrap(), before);
}

#[test]
fn delete_removes_the_host_and_is_idempotent() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    let id = store.data.hosts[0].id.clone();
    store.delete(&id).unwrap();
    assert!(store.data.hosts.is_empty());
    assert!(
        Store::open(store.path.clone())
            .unwrap()
            .data
            .hosts
            .is_empty()
    );

    store.delete(&id).unwrap();
    assert!(store.data.hosts.is_empty());
    store.delete("missing-id").unwrap();
    assert_eq!(Store::open(store.path.clone()).unwrap().data.version, 2);
}

#[test]
fn trust_pins_fingerprints_per_host_and_port() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.trust("example.invalid", 22, "SHA256:first").unwrap();
    store.trust("example.invalid", 22, "SHA256:first").unwrap();
    store
        .trust("example.invalid", 2222, "SHA256:other-port")
        .unwrap();
    store
        .trust("other.invalid", 22, "SHA256:other-host")
        .unwrap();

    assert!(
        store
            .trust("example.invalid", 22, "SHA256:changed")
            .is_err()
    );
    assert_eq!(
        store
            .data
            .known_hosts
            .get(&store::key("example.invalid", 22))
            .map(String::as_str),
        Some("SHA256:first")
    );

    let mut reopened = Store::open(store.path.clone()).unwrap();
    assert_eq!(reopened.data.known_hosts.len(), 3);
    assert!(
        reopened
            .trust("example.invalid", 22, "SHA256:changed")
            .is_err()
    );
}

#[test]
fn trust_rejects_empty_hosts_and_fingerprints() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    assert!(store.trust("", 22, "SHA256:test").is_err());
    assert!(store.trust("   ", 22, "SHA256:test").is_err());
    assert!(store.trust("example.invalid", 22, "").is_err());
    assert!(store.trust("example.invalid", 22, "   ").is_err());
    assert!(store.data.known_hosts.is_empty());
}

#[test]
fn export_excludes_secrets_and_known_hosts_and_can_be_imported() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    store
        .trust("example.invalid", 22, "SHA256:secret-fingerprint")
        .unwrap();
    #[cfg(windows)]
    {
        let mut p = store.data.hosts[0].clone();
        store.save(p.clone(), Some("export-secret"), true).unwrap();
        p = store.data.hosts[0].clone();
        assert!(p.encrypted_secret.is_some());
    }

    let export = dir.path().join("hosts.json");
    store.export(&export).unwrap();
    let text = fs::read_to_string(&export).unwrap();
    assert!(!text.contains("export-secret"));
    assert!(!text.contains("secret-fingerprint"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(value["hosts"].as_array().unwrap().len(), 1);
    assert!(value["hosts"][0]["encryptedSecret"].is_null());
    assert_eq!(value["knownHosts"].as_object().unwrap().len(), 0);

    let mut fresh = Store::open(dir.path().join("fresh.json")).unwrap();
    assert_eq!(fresh.import(&export).unwrap(), 1);
    assert!(fresh.data.hosts[0].encrypted_secret.is_none());
    assert!(fresh.data.known_hosts.is_empty());
}

#[test]
fn export_refuses_to_overwrite_the_live_configuration() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    store.trust("example.invalid", 22, "SHA256:keep").unwrap();
    let path = store.path.clone();

    let before = fs::read(&path).unwrap();
    assert!(store.export(&path).is_err());
    fs::create_dir(dir.path().join("nested")).unwrap();
    let alias = dir
        .path()
        .join("nested")
        .join("..")
        .join("opentermius-native.json");
    assert!(store.export(&alias).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.known_hosts.len(), 1);
    #[cfg(windows)]
    {
        assert!(reopened.data.hosts[0].encrypted_secret.is_none());
    }
}

#[test]
fn import_regenerates_ids_and_never_overwrites_existing_hosts() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    let mut existing = profile();
    existing.id = "fixed-id".into();
    existing.name = "Existing".into();
    store.save(existing, None, false).unwrap();

    let import = dir.path().join("import.json");
    write_json(
        &import,
        r#"{
            "version": 2,
            "knownHosts": {"existing.invalid:22": "SHA256:evil"},
            "hosts": [{
                "id": "fixed-id",
                "name": "Imported",
                "host": "import.invalid",
                "username": "other",
                "encryptedSecret": "never-import-this"
            }]
        }"#,
    );
    assert_eq!(store.import(&import).unwrap(), 1);
    assert_eq!(store.data.hosts.len(), 2);
    assert_eq!(store.data.hosts[0].name, "Existing");
    assert_eq!(store.data.hosts[0].host, "example.invalid");
    assert_eq!(store.data.hosts[0].id, "fixed-id");
    assert_eq!(store.data.hosts[1].name, "Imported");
    assert_ne!(store.data.hosts[1].id, "fixed-id");
    assert!(!store.data.hosts[1].id.is_empty());
    assert!(store.data.hosts[1].encrypted_secret.is_none());
    assert!(
        store.data.known_hosts.is_empty(),
        "import must not inject trust anchors"
    );
}

#[test]
fn import_assigns_distinct_ids_to_duplicate_entries() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    let import = dir.path().join("import.json");
    write_json(
        &import,
        r#"{"version":2,"hosts":[
            {"id":"same","name":"A","host":"example.invalid","username":"u"},
            {"id":"same","name":"A","host":"example.invalid","username":"u"}
        ]}"#,
    );
    assert_eq!(store.import(&import).unwrap(), 2);
    assert_eq!(store.data.hosts.len(), 2);
    assert_ne!(store.data.hosts[0].id, store.data.hosts[1].id);
    assert_ne!(store.data.hosts[0].id, "same");
    assert_ne!(store.data.hosts[1].id, "same");
}

#[test]
fn import_rejects_malformed_missing_and_invalid_payloads_without_mutation() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    let cases = [
        "{ definitely not json".to_string(),
        r#"{"version":2}"#.to_string(),
        r#"{"version":2,"hosts":{}}"#.to_string(),
        r#"{"version":2,"hosts":[{"name":"x","host":"h","username":"u","port":0}]}"#.to_string(),
        r#"{"version":2,"hosts":[{"name":"x","host":"h","username":"u","authType":"key"}]}"#
            .to_string(),
        format!(
            r#"{{"version":2,"hosts":[{{"name":"{}","host":"h","username":"u"}}]}}"#,
            "n".repeat(129)
        ),
        r#"{"version":2,"hosts":[{"name":"x","host":"bad host","username":"u"}]}"#.to_string(),
    ];
    for (index, text) in cases.iter().enumerate() {
        let path = dir.path().join(format!("bad-{index}.json"));
        write_json(&path, text);
        assert!(store.import(&path).is_err(), "case {index} should fail");
        assert!(store.data.hosts.is_empty(), "case {index} mutated hosts");
    }
}

#[test]
fn import_rejects_oversized_files_and_too_many_hosts() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);

    let oversized = dir.path().join("oversized.json");
    let file = fs::File::create(&oversized).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    assert!(store.import(&oversized).is_err());

    let many = dir.path().join("many.json");
    let mut text = String::from(r#"{"version":2,"hosts":["#);
    for index in 0..10001 {
        if index > 0 {
            text.push(',');
        }
        text.push_str(r#"{"name":"h","host":"h","username":"u"}"#);
    }
    text.push_str("]}");
    write_json(&many, &text);
    assert!(store.import(&many).is_err());
    assert!(store.data.hosts.is_empty());
}

#[test]
fn key_is_stable_and_distinguishes_host_and_port() {
    assert_eq!(
        store::key("example.invalid", 22),
        store::key("example.invalid", 22)
    );
    assert_ne!(
        store::key("example.invalid", 22),
        store::key("example.invalid", 23)
    );
    assert_ne!(
        store::key("example.invalid", 22),
        store::key("EXAMPLE.invalid", 22)
    );
    let key = store::key("例子.测试", 22);
    assert!(key.contains("例子.测试"));
}

#[cfg(windows)]
#[test]
fn dpapi_roundtrips_unicode_empty_and_long_values() {
    for secret in ["密码 🔐", "", "a", &"x".repeat(16384)] {
        let encrypted = store::protect(secret).unwrap();
        assert_ne!(encrypted.as_str(), secret);
        assert_eq!(store::unprotect(&encrypted).unwrap(), secret);
    }
    assert!(
        !store::protect("plain-secret")
            .unwrap()
            .contains("plain-secret")
    );
}

#[cfg(windows)]
#[test]
fn dpapi_rejects_invalid_and_tampered_ciphertexts() {
    assert!(store::unprotect("not-base64!").is_err());
    assert!(store::unprotect("").is_err());

    let encrypted = store::protect("tamper-me").unwrap();
    let mut bytes = STANDARD.decode(&encrypted).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x40;
    let tampered = STANDARD.encode(bytes);
    assert!(store::unprotect(&tampered).is_err());

    assert!(store::protect(&"x".repeat(16385)).is_err());
}

#[test]
fn persist_refuses_to_write_a_configuration_that_could_not_be_reopened() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    store.save(profile(), None, false).unwrap();
    let before = fs::read(&store.path).unwrap();
    let mut huge = profile();
    huge.auth_type = "key".into();
    huge.private_key_path = format!(r"C:\keys\{}", "k".repeat(30_000));
    for _ in 0..560 {
        let mut p = huge.clone();
        p.validate().unwrap();
        store.data.hosts.push(p);
    }

    assert!(
        store.persist().is_err(),
        "oversized writes must be rejected"
    );
    assert_eq!(
        fs::read(&store.path).unwrap(),
        before,
        "a rejected write must preserve the saved config"
    );
    assert_eq!(Store::open(store.path.clone()).unwrap().data.hosts.len(), 1);
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
}

#[test]
fn a_valid_export_larger_than_five_megabytes_can_be_imported_again() {
    let dir = tempdir().unwrap();
    let mut store = store_in(&dir);
    let long_path = format!(r"C:\keys\{}", "k".repeat(30_000));
    for index in 0..220 {
        let mut p = profile();
        p.host = format!("host-{index}.invalid");
        p.name = format!("Host {index}");
        p.auth_type = "key".into();
        p.private_key_path = long_path.clone();
        store.data.hosts.push(p);
    }
    store.persist().unwrap();

    let export = dir.path().join("large-export.json");
    store.export(&export).unwrap();
    assert!(fs::metadata(&export).unwrap().len() > 5 * 1024 * 1024);

    let mut fresh = Store::open(dir.path().join("fresh.json")).unwrap();
    assert_eq!(fresh.import(&export).unwrap(), 220);
    assert_eq!(fresh.data.hosts.len(), 220);
}
