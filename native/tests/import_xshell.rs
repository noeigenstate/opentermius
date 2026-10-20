//! All fixtures are handwritten synthetic sessions; no user sessions are used.
use opentermius::{
    import_xshell::{self, ImportSummary, ScanResult},
    store::Store,
};
use std::{fs, path::Path};

fn session(protocol: &str, host: &str, extra: &str, auth: &str) -> String {
    format!(
        "[CONNECTION]\r\nHost={host}\r\nProtocol={protocol}\r\n{extra}[CONNECTION:AUTHENTICATION]\r\nUserName=tester\r\n{auth}"
    )
}

fn write(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn utf16_unicode_names_nested_groups_and_default_port() {
    let dir = tempfile::tempdir().unwrap();
    let text = session("SSH", "10.0.0.1", "", "Method=0\r\n");
    let bytes: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    write(dir.path(), "开发/测试/中文会话.xsh", bytes);
    let result = import_xshell::scan(dir.path());
    assert!(result.skipped.is_empty());
    let profile = &result.hosts[0].profile;
    assert_eq!(profile.name, "中文会话");
    assert_eq!(profile.group, "开发/测试");
    assert_eq!(profile.port, 22);
    assert_eq!(profile.host, "10.0.0.1");
    assert_eq!(profile.username, "tester");
}

#[test]
fn utf8_with_and_without_bom_support_sftp_and_case_insensitive_fields() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "plain.xsh",
        session("SFTP", "example.internal", "Port=2222\r\n", "Method=2\r\n"),
    );
    write(
        dir.path(),
        "bom.XSH",
        "\u{feff}[connection]\r\nhost=10.0.0.1\r\nprotocol=ssh\r\n[connection:authentication]\r\nusername=tester\r\n",
    );
    let result = import_xshell::scan(dir.path());
    assert!(result.skipped.is_empty());
    assert_eq!(result.hosts.len(), 2);
    assert!(
        result
            .hosts
            .iter()
            .all(|draft| draft.profile.group.is_empty() && draft.profile.auth_type == "password")
    );
    assert_eq!(
        result
            .hosts
            .iter()
            .find(|draft| draft.profile.name == "plain")
            .unwrap()
            .profile
            .port,
        2222
    );
}

#[test]
fn unsupported_protocols_and_malformed_sessions_have_safe_reasons() {
    let dir = tempfile::tempdir().unwrap();
    for protocol in ["TELNET", "RLOGIN", "SERIAL", "LOCAL"] {
        write(
            dir.path(),
            &format!("{protocol}.xsh"),
            session(protocol, "10.0.0.1", "", ""),
        );
    }
    for (name, text) in [
        ("broken", "not an INI session".to_string()),
        (
            "bad-port",
            session("SSH", "10.0.0.1", "Port=secret-marker\r\n", ""),
        ),
        ("zero-port", session("SSH", "10.0.0.1", "Port=0\r\n", "")),
        ("missing-host", session("SSH", "", "", "")),
        (
            "bad-section",
            "[CONNECTION\r\nPassword=secret-marker".into(),
        ),
        (
            "bad-field",
            session("SSH", "10.0.0.1", "secret-marker\r\n", ""),
        ),
        ("gssapi", session("SSH", "10.0.0.1", "", "Method=3\r\n")),
    ] {
        write(dir.path(), &format!("{name}.xsh"), text);
    }
    write(dir.path(), "invalid-utf8.xsh", [0xff, 0x00]);
    write(dir.path(), "truncated-utf16.xsh", [0xff, 0xfe, 0x00]);
    write(dir.path(), "invalid-utf16.xsh", [0xff, 0xfe, 0x00, 0xd8]);
    write(dir.path(), "ignored.txt", "this is not a session");
    let result = import_xshell::scan(dir.path());
    assert!(result.hosts.is_empty());
    assert_eq!(result.skipped.len(), 14);
    assert!(
        result
            .skipped
            .iter()
            .all(|(_, reason)| !reason.is_empty() && !reason.contains("secret-marker"))
    );
}

#[test]
fn passwords_passphrases_and_key_names_never_enter_profiles_or_persistence() {
    let dir = tempfile::tempdir().unwrap();
    for (name, auth) in [
        ("password", "Method=0\r\n"),
        ("public-key", "Method=1\r\nUserKey=fake-key-marker\r\n"),
        ("legacy-usekey", "UseKey=1\r\n"),
        ("legacy-userkey", "UserKey=fake-key-marker\r\n"),
    ] {
        write(
            dir.path(),
            &format!("{name}.xsh"),
            session(
                "SSH",
                &format!("{name}.internal"),
                "",
                &format!(
                    "{auth}Password=fake-password-marker\r\nPassphrase=fake-passphrase-marker\r\n"
                ),
            ),
        );
    }
    let result = import_xshell::scan(dir.path());
    assert_eq!(result.hosts.len(), 4);
    assert_eq!(
        result.hosts.iter().filter(|draft| draft.needs_key).count(),
        3
    );
    for draft in &result.hosts {
        let mut profile = draft.profile.clone();
        assert!(profile.validate().is_ok());
        assert_eq!(profile.auth_type, "password");
        assert!(profile.private_key_path.is_empty());
        assert!(profile.encrypted_secret.is_none());
    }
    let mut store = Store::open(dir.path().join("config.json")).unwrap();
    let summary = import_xshell::merge(&mut store, result).unwrap();
    assert_eq!(summary.imported, 4);
    assert_eq!(summary.needs_key, 3);
    assert!(summary.message().contains("选择私钥"));
    let json = fs::read_to_string(&store.path).unwrap();
    for marker in [
        "fake-password-marker",
        "fake-passphrase-marker",
        "fake-key-marker",
    ] {
        assert!(!json.contains(marker));
    }
    assert_eq!(Store::open(store.path.clone()).unwrap().data.hosts.len(), 4);
}

#[test]
fn merge_skips_existing_and_batch_duplicates_but_preserves_distinct_ports_and_users() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("Sessions");
    write(
        &sessions,
        "a.xsh",
        session("SSH", "EXAMPLE.internal", "", ""),
    );
    write(
        &sessions,
        "b.xsh",
        session("SSH", "example.internal", "", ""),
    );
    write(
        &sessions,
        "c.xsh",
        session("SSH", "example.internal", "Port=2222\r\n", ""),
    );
    write(
        &sessions,
        "d.xsh",
        session("SSH", "example.internal", "", "UserName=tester2\r\n"),
    );
    let mut store = Store::open(dir.path().join("config.json")).unwrap();
    let first = import_xshell::merge(&mut store, import_xshell::scan(&sessions)).unwrap();
    assert_eq!(
        first,
        ImportSummary {
            imported: 3,
            duplicates: 1,
            ..Default::default()
        }
    );
    let previous = fs::read(&store.path).unwrap();
    let second = import_xshell::merge(&mut store, import_xshell::scan(&sessions)).unwrap();
    assert_eq!(
        second,
        ImportSummary {
            imported: 0,
            duplicates: 4,
            ..Default::default()
        }
    );
    assert_eq!(fs::read(&store.path).unwrap(), previous);
    assert_eq!(store.data.hosts.len(), 3);
}

#[test]
fn merge_revalidates_drafts_and_rolls_back_if_persist_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.xsh",
        session("SSH", "10.0.0.1", "", ""),
    );
    let mut result = import_xshell::scan(dir.path());
    let mut invalid = result.hosts[0].clone();
    invalid.profile.port = 0;
    result.hosts.push(invalid);
    let mut store = Store::open(dir.path().join("config.json")).unwrap();
    let summary = import_xshell::merge(&mut store, result).unwrap();
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 1);
    let previous_id = store.data.hosts[0].id.clone();
    let blocker = dir.path().join("not-a-directory");
    fs::write(&blocker, "synthetic blocker").unwrap();
    store.path = blocker.join("config.json");
    let mut another = import_xshell::scan(dir.path());
    another.hosts[0].profile.host = "other.internal".into();
    assert!(import_xshell::merge(&mut store, another).is_err());
    assert_eq!(store.data.hosts.len(), 1);
    assert_eq!(store.data.hosts[0].id, previous_id);
}

#[test]
fn missing_directory_and_empty_batch_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let scanned = import_xshell::scan(dir.path().join("missing"));
    assert_eq!(scanned.skipped.len(), 1);
    let mut store = Store::open(dir.path().join("config.json")).unwrap();
    assert_eq!(
        import_xshell::merge(&mut store, scanned).unwrap().skipped,
        1
    );
    assert_eq!(
        import_xshell::merge(&mut store, ScanResult::default()).unwrap(),
        ImportSummary::default()
    );
    assert!(!store.path.exists());
}
