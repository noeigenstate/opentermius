//! Local WindTerm session import. WindTerm keeps sessions in
//! `<root>/.wind/profiles/<profile>/terminal/user.sessions` (a JSON array), where `<root>`
//! is the install folder for portable setups (`profiles.config` → `"path": "."`) or the
//! user's home. Only SSH sessions are imported; `session.autoLogin` holds WindTerm's
//! encrypted credentials and is never read, so usernames come only from `user@host` targets.
use crate::import_xshell::{HostDraft, ScanResult};
use crate::store::Profile;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_SESSIONS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SESSIONS: usize = 10_000;

/// Parses a `user.sessions` document. `home` expands WindTerm's `~` in key paths.
pub fn parse(json: &str, home: &Path) -> Result<ScanResult> {
    let sessions: Vec<Value> = serde_json::from_str(json.trim_start_matches('\u{feff}'))
        .context(crate::i18n::t("Not a WindTerm sessions file"))?;
    let mut result = ScanResult::default();
    for (index, session) in sessions.iter().take(MAX_SESSIONS).enumerate() {
        let text = |key: &str| session.get(key).and_then(Value::as_str).unwrap_or_default().trim();
        if !text("session.protocol").eq_ignore_ascii_case("ssh") {
            continue;
        }
        match draft(session, home) {
            Ok(draft) => result.hosts.push(draft),
            Err(error) => result.skipped.push((PathBuf::from(format!("#{}", index + 1)), error.to_string())),
        }
    }
    Ok(result)
}

fn draft(session: &Value, home: &Path) -> Result<HostDraft> {
    let text = |key: &str| session.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let target = text("session.target");
    let (username, host) = match target.rsplit_once('@') {
        Some((user, host)) => (user.to_string(), host.to_string()),
        None => (String::new(), target),
    };
    let port = match session.get("session.port") {
        None | Some(Value::Null) => 22,
        Some(value) => value
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port > 0)
            .context(crate::i18n::t("Port must be between 1 and 65535"))?,
    };
    let key = text("ssh.identityFilePath.windows");
    let key = if key.is_empty() { text("ssh.identityFilePath") } else { key };
    let key_path = expand_home(&key, home);
    let key_found = key_path.as_ref().is_some_and(|path| path.is_file());
    let mut profile = Profile {
        id: String::new(),
        name: text("session.label"),
        host,
        port,
        username,
        auth_type: if key_found { "key" } else { "password" }.into(),
        private_key_path: if key_found { key_path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default() } else { String::new() },
        key_id: String::new(),
        // WindTerm nests groups with '>'; OpenTermius uses '/'.
        group: text("session.group").split('>').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/"),
        encrypted_secret: None,
        tags: Vec::new(),
        last_connected: None,
        created_at: None,
    };
    profile.validate()?;
    Ok(HostDraft { profile, needs_key: !key.is_empty() && !key_found })
}

fn expand_home(path: &str, home: &Path) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    match path.strip_prefix('~') {
        Some(rest) => Some(home.join(rest.trim_start_matches(['/', '\\']))),
        None => Some(PathBuf::from(path)),
    }
}

/// Reads one `user.sessions` file with a size limit.
pub fn scan_file(path: &Path, home: &Path) -> Result<ScanResult> {
    let file = fs::File::open(path).context(crate::i18n::t("Cannot read the WindTerm sessions file"))?;
    let mut text = String::new();
    file.take(MAX_SESSIONS_BYTES + 1)
        .read_to_string(&mut text)
        .context(crate::i18n::t("Cannot read the WindTerm sessions file"))?;
    ensure!(text.len() as u64 <= MAX_SESSIONS_BYTES, "{}", crate::i18n::t("The WindTerm sessions file is too large"));
    parse(&text, home)
}

/// Every `user.sessions` reachable from `root`: a WindTerm install folder, its `.wind`
/// folder, a profile folder, or the sessions file itself.
pub fn find_sessions(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    let mut bases = vec![root.to_path_buf()];
    if let Some(configured) = profiles_root(root) {
        bases.insert(0, configured);
    }
    let mut found = Vec::new();
    for base in bases {
        for profiles in [base.join(".wind").join("profiles"), base.join("profiles"), base.clone()] {
            let direct = profiles.join("terminal").join("user.sessions");
            if direct.is_file() {
                found.push(direct);
            }
            if let Ok(entries) = fs::read_dir(&profiles) {
                for entry in entries.flatten().take(256) {
                    let file = entry.path().join("terminal").join("user.sessions");
                    if file.is_file() {
                        found.push(file);
                    }
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `profiles.config` → `{"path": "."}` places the profiles relative to the install folder.
fn profiles_root(install: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(install.join("profiles.config")).ok()?;
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let path = value.get("path")?.as_str()?.trim();
    (!path.is_empty()).then(|| install.join(path))
}

/// Likely WindTerm locations: `~/.wind` and WindTerm folders at the top of each drive or
/// in Program Files. Only folder names are listed; no session is read here.
pub fn default_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![home.to_path_buf()];
    #[cfg(windows)]
    {
        let mut parents: Vec<PathBuf> = (b'C'..=b'Z').map(|d| PathBuf::from(format!("{}:\\", d as char))).filter(|p| p.is_dir()).collect();
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(dir) = std::env::var_os(var) {
                parents.push(PathBuf::from(dir));
            }
        }
        for parent in parents {
            if let Ok(entries) = fs::read_dir(&parent) {
                for entry in entries.flatten().take(4096) {
                    if entry.file_name().to_string_lossy().to_ascii_lowercase().contains("windterm") && entry.path().is_dir() {
                        roots.push(entry.path());
                    }
                }
            }
        }
    }
    roots
}

/// All sessions files found in the default locations.
pub fn detect(home: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = default_roots(home).iter().flat_map(|root| find_sessions(root)).collect();
    found.sort();
    found.dedup();
    found
}
