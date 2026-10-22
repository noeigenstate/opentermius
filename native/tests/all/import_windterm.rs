//! All fixtures are handwritten synthetic WindTerm sessions; no user sessions are used.
use opentermius::{import_windterm, import_xshell, store::Store};
use std::{fs, path::Path};

const SESSIONS: &str = r#"[
    {
        "session.autoLogin" : "ZmFrZS1lbmNyeXB0ZWQtY3JlZGVudGlhbHMtbWFya2Vy",
        "session.icon" : "session::square-mediumorchid",
        "session.label" : "Build box",
        "session.port" : 2222,
        "session.protocol" : "SSH",
        "session.target" : "10.0.0.5",
        "session.uuid" : "00000000-0000-0000-0000-000000000001"
    },
    {
        "session.group" : "Work>Staging",
        "session.label" : "部署机",
        "session.protocol" : "SSH",
        "session.target" : "deploy@staging.example.invalid",
        "session.uuid" : "00000000-0000-0000-0000-000000000002",
        "ssh.identityFilePath.windows" : "~\\.ssh\\id_test"
    },
    {
        "session.label" : "Missing key",
        "session.protocol" : "SSH",
        "session.target" : "db.example.invalid",
        "ssh.identityFilePath.windows" : "~\\.ssh\\absent_key"
    },
    {
        "process.arguments" : "",
        "session.label" : "PowerShell",
        "session.protocol" : "Shell",
        "session.target" : "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
    },
    { "session.label" : "Bad port", "session.protocol" : "SSH", "session.target" : "x.example.invalid", "session.port" : 70000 },
    { "session.label" : "No host", "session.protocol" : "SSH", "session.target" : "" }
]"#;

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn imports_ssh_sessions_only_without_credentials() {
    let home = tempfile::tempdir().unwrap();
    write(home.path(), ".ssh/id_test", "synthetic key placeholder");
    let result = import_windterm::parse(SESSIONS, home.path()).unwrap();
    // Shell sessions are not hosts; invalid SSH entries are reported as skipped.
    assert_eq!(result.hosts.len(), 3);
    assert_eq!(result.skipped.len(), 2);

    let build = &result.hosts[0].profile;
    assert_eq!((build.name.as_str(), build.host.as_str(), build.port), ("Build box", "10.0.0.5", 2222));
    assert!(build.username.is_empty());
    assert_eq!(build.auth_type, "password");
    assert!(build.encrypted_secret.is_none());

    let deploy = &result.hosts[1].profile;
    assert_eq!((deploy.username.as_str(), deploy.host.as_str(), deploy.port), ("deploy", "staging.example.invalid", 22));
    assert_eq!(deploy.group, "Work/Staging");
    assert_eq!(deploy.auth_type, "key");
    assert_eq!(Path::new(&deploy.private_key_path), home.path().join(".ssh").join("id_test"));
    assert!(!result.hosts[1].needs_key);

    let missing = &result.hosts[2];
    assert_eq!(missing.profile.auth_type, "password");
    assert!(missing.profile.private_key_path.is_empty());
    assert!(missing.needs_key);
}

#[test]
fn merge_dedupes_and_never_stores_windterm_secrets() {
    let home = tempfile::tempdir().unwrap();
    let mut store = Store::open(home.path().join("config.json")).unwrap();
    let first = import_xshell::merge(&mut store, import_windterm::parse(SESSIONS, home.path()).unwrap()).unwrap();
    assert_eq!((first.imported, first.duplicates, first.skipped), (3, 0, 2));
    let second = import_xshell::merge(&mut store, import_windterm::parse(SESSIONS, home.path()).unwrap()).unwrap();
    assert_eq!((second.imported, second.duplicates), (0, 3));
    let json = fs::read_to_string(&store.path).unwrap();
    assert!(!json.contains("ZmFrZS1lbmNyeXB0ZWQtY3JlZGVudGlhbHMtbWFya2Vy"));
    assert_eq!(Store::open(store.path.clone()).unwrap().data.hosts.len(), 3);
}

#[test]
fn finds_portable_and_home_profiles() {
    let install = tempfile::tempdir().unwrap();
    write(install.path(), "profiles.config", "{\n    \"path\" : \".\"\n}\n");
    write(install.path(), ".wind/profiles/default.v10/terminal/user.sessions", "[]");
    write(install.path(), ".wind/profiles/work.v10/terminal/user.sessions", "[]");
    let found = import_windterm::find_sessions(install.path());
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|p| p.ends_with("terminal/user.sessions") || p.ends_with("terminal\\user.sessions")));
    // The .wind folder, a profile folder or the file itself also work.
    assert_eq!(import_windterm::find_sessions(&install.path().join(".wind")).len(), 2);
    assert_eq!(import_windterm::find_sessions(&install.path().join(".wind/profiles/default.v10")).len(), 1);
    assert_eq!(import_windterm::find_sessions(&found[0]), vec![found[0].clone()]);

    let home = tempfile::tempdir().unwrap();
    write(home.path(), ".wind/profiles/default.v10/terminal/user.sessions", "[]");
    assert!(import_windterm::detect(home.path()).contains(&home.path().join(".wind/profiles/default.v10/terminal/user.sessions")));
    assert!(import_windterm::find_sessions(&home.path().join("missing")).is_empty());
}

#[test]
fn rejects_files_that_are_not_windterm_sessions() {
    let home = tempfile::tempdir().unwrap();
    assert!(import_windterm::parse("not json", home.path()).is_err());
    assert!(import_windterm::parse("{\"session.target\": \"x\"}", home.path()).is_err());
    let bom = format!("\u{feff}{SESSIONS}");
    assert_eq!(import_windterm::parse(&bom, home.path()).unwrap().hosts.len(), 3);
}

/// Opt-in check against a real install: `OPENTERMIUS_WINDTERM_DIR=<windterm folder>`.
/// Prints counts only, never session contents.
#[test]
#[ignore]
fn real_windterm_install_counts() {
    let Some(root) = std::env::var_os("OPENTERMIUS_WINDTERM_DIR") else { return };
    let home = tempfile::tempdir().unwrap();
    let files = import_windterm::find_sessions(Path::new(&root));
    assert!(!files.is_empty(), "no user.sessions found");
    for file in &files {
        let result = import_windterm::scan_file(file, home.path()).unwrap();
        let usernames = result.hosts.iter().filter(|h| !h.profile.username.is_empty()).count();
        let keys = result.hosts.iter().filter(|h| h.profile.auth_type == "key" || h.needs_key).count();
        println!("sessions file: {} ssh hosts, {} skipped, {} with username, {} with key", result.hosts.len(), result.skipped.len(), usernames, keys);
        for (_, reason) in &result.skipped {
            println!("skipped: {reason}");
        }
    }
    let detected = import_windterm::detect(&dirs::home_dir().unwrap());
    println!("auto-detected sessions files: {}", detected.len());
}
