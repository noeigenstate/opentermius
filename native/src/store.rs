use anyhow::{Context, Result, ensure};
#[cfg(windows)]
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[cfg(any(not(windows), test))]
mod secrets;
#[cfg(unix)]
pub use secrets::configure_directory as configure_secret_directory;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "ssh_port")]
    pub port: u16,
    pub username: String,
    #[serde(default = "password_auth")]
    pub auth_type: String,
    #[serde(default)]
    pub private_key_path: String,
    /// Keychain key used for "key" authentication; takes precedence over the
    /// file path when set.
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub encrypted_secret: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default)]
    pub last_connected: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}
fn ssh_port() -> u16 {
    22
}
fn password_auth() -> String {
    "password".into()
}
impl Profile {
    pub fn validate(&mut self) -> Result<()> {
        for (value, label, max) in [
            (&mut self.name, crate::i18n::t("Name"), 128),
            (&mut self.host, crate::i18n::t("Address"), 253),
            (&mut self.username, crate::i18n::t("Username"), 128),
            (&mut self.group, crate::i18n::t("Group"), 128),
        ] {
            *value = value.trim().into();
            ensure!(value.chars().count() <= max
                    && value.len() <= max * 4
                    && !value.chars().any(char::is_control), "{}", crate::tf!("Invalid {label}", "label" => format!("{}", label)));
        }
        ensure!(!self.host.is_empty(), "{}", crate::i18n::t("Host address is required"));
        ensure!(!self.host.chars().any(char::is_whitespace), "{}", crate::i18n::t("Invalid address"));
        if self.name.is_empty() {
            self.name = self.host.chars().take(128).collect();
        }
        ensure!(self.port > 0, "{}", crate::i18n::t("Port must be between 1 and 65535"));
        ensure!(self.auth_type == "password" || self.auth_type == "key", "{}", crate::i18n::t("Invalid authentication method"));
        self.private_key_path = self.private_key_path.trim().into();
        ensure!(self.private_key_path.chars().count() <= 32767
                && self.private_key_path.len() <= 32767 * 4
                && !self.private_key_path.chars().any(char::is_control), "{}", crate::i18n::t("Invalid private key path"));
        ensure!(self.auth_type != "key"
                || !self.private_key_path.is_empty()
                || !self.key_id.is_empty(), "{}", crate::i18n::t("Select a private key file or a Keychain key"));
        if self.id.is_empty() {
            self.id = Uuid::new_v4().to_string();
        }
        self.tags = crate::hosts::normalize_tags(&self.tags);
        for (value, label) in [
            (&self.created_at, crate::i18n::t("Creation time")),
            (&self.last_connected, crate::i18n::t("Last connection time")),
        ] {
            if let Some(value) = value {
                ensure!(value.len() <= 64 && chrono::DateTime::parse_from_rfc3339(value).is_ok(), "{}", crate::tf!("Invalid {label}", "label" => format!("{}", label)));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub version: u32,
    pub hosts: Vec<Profile>,
    #[serde(default)]
    pub known_hosts: BTreeMap<String, String>,
    #[serde(default)]
    pub keys: Vec<crate::keychain::KeyEntry>,
    #[serde(default)]
    pub snippets: Vec<crate::snippets::Snippet>,
    #[serde(default)]
    pub settings: crate::settings::Settings,
    #[serde(default)]
    pub hosts_view: crate::hosts::HostsView,
}
pub struct Store {
    pub path: PathBuf,
    pub data: Data,
}
impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            ensure!(fs::metadata(&path)?.len() < 16 * 1024 * 1024, "{}", crate::i18n::t("Local configuration file is too large"));
            let mut data: Data = serde_json::from_slice(&fs::read(&path)?)
                .context(crate::i18n::t("Local configuration is corrupted; keep the file and restore a backup"))?;
            ensure!(data.version == 2, "{}", crate::i18n::t("Unsupported local configuration version"));
            for p in &mut data.hosts {
                p.validate()?;
            }
            data
        } else {
            Data {
                version: 2,
                ..Data::default()
            }
        };
        Ok(Self { path, data })
    }
    pub fn persist(&self) -> Result<()> {
        let parent = self.path.parent().context(crate::i18n::t("Invalid data directory"))?;
        let encoded = serde_json::to_vec_pretty(&self.data)?;
        ensure!(encoded.len() < 16 * 1024 * 1024, "{}", crate::i18n::t("Local configuration file is too large"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!("opentermius-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&encoded)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    pub fn save(
        &mut self,
        mut profile: Profile,
        secret: Option<&str>,
        remember: bool,
    ) -> Result<()> {
        profile.validate()?;
        ensure!(profile.key_id.is_empty()
                || profile.auth_type != "key"
                || self.data.keys.iter().any(|key| key.id == profile.key_id), "{}", crate::i18n::t("Selected Keychain key does not exist"));
        let old = self.data.clone();
        let previous = self.data.hosts.iter().find(|p| p.id == profile.id);
        let previous_created = previous.and_then(|p| p.created_at.clone());
        let previous_connected = previous.and_then(|p| p.last_connected.clone());
        let previous_os = previous.and_then(|p| p.os.clone());
        profile.encrypted_secret = if remember {
            if let Some(value) = secret.filter(|s| !s.is_empty()) {
                Some(protect(value)?)
            } else {
                previous
                    .filter(|p| p.auth_type == profile.auth_type)
                    .and_then(|p| p.encrypted_secret.clone())
            }
        } else {
            None
        };
        profile.created_at = profile
            .created_at
            .or(previous_created)
            .or_else(|| Some(crate::hosts::now_rfc3339()));
        if profile.last_connected.is_none() {
            profile.last_connected = previous_connected;
        }
        if profile.os.is_none() {
            profile.os = previous_os;
        }
        match self.data.hosts.iter_mut().find(|p| p.id == profile.id) {
            Some(p) => *p = profile,
            None => self.data.hosts.push(profile),
        }
        if let Err(error) = self.persist() {
            self.data = old;
            return Err(error);
        }
        Ok(())
    }
    pub fn delete(&mut self, id: &str) -> Result<()> {
        let old = self.data.clone();
        self.data.hosts.retain(|p| p.id != id);
        if let Err(error) = self.persist() {
            self.data = old;
            return Err(error);
        }
        Ok(())
    }
    pub fn trust(&mut self, host: &str, port: u16, fingerprint: &str) -> Result<()> {
        ensure!(!host.trim().is_empty()
                && !host.chars().any(|c| c.is_whitespace() || c.is_control())
                && host.len() <= 253 * 4, "{}", crate::i18n::t("Invalid host address"));
        ensure!(!fingerprint.trim().is_empty()
                && !fingerprint.chars().any(|c| c.is_whitespace() || c.is_control())
                && fingerprint.len() <= 512, "{}", crate::i18n::t("Invalid host fingerprint"));
        let key = key(host, port);
        if let Some(previous) = self.data.known_hosts.get(&key) {
            ensure!(previous == fingerprint, "{}", crate::i18n::t("SSH host key changed; connection refused"));
            return Ok(());
        }
        self.data
            .known_hosts
            .insert(key.clone(), fingerprint.into());
        if let Err(error) = self.persist() {
            self.data.known_hosts.remove(&key);
            return Err(error);
        }
        Ok(())
    }
    pub fn export(&self, path: &Path) -> Result<()> {
        ensure!(path != self.path, "{}", crate::i18n::t("Cannot export to the current configuration file"));
        if path.exists() && self.path.exists() {
            ensure!(fs::canonicalize(path)? != fs::canonicalize(&self.path)?, "{}", crate::i18n::t("Cannot export to the current configuration file"));
        }
        let mut data = self.data.clone();
        data.known_hosts.clear();
        // Host export must not carry keychain private material or saved
        // credentials; hosts keep their key id but the key must be re-imported.
        data.keys.clear();
        for p in &mut data.hosts {
            p.encrypted_secret = None;
        }
        fs::write(path, serde_json::to_vec_pretty(&data)?)?;
        Ok(())
    }
    pub fn import(&mut self, path: &Path) -> Result<usize> {
        ensure!(fs::metadata(path)?.len() < 16 * 1024 * 1024, "{}", crate::i18n::t("Import file must not exceed 16 MB"));
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let hosts = value
            .get("hosts")
            .and_then(|v| v.as_array())
            .context(crate::i18n::t("Host list is missing"))?;
        ensure!(hosts.len() <= 10000, "{}", crate::i18n::t("Too many hosts"));
        let mut imported = Vec::new();
        for item in hosts {
            let mut p: Profile = serde_json::from_value(item.clone())?;
            p.id = Uuid::new_v4().to_string();
            p.encrypted_secret = None;
            p.validate()?;
            if p.created_at.is_none() {
                p.created_at = Some(crate::hosts::now_rfc3339());
            }
            imported.push(p);
        }
        let count = imported.len();
        let old = self.data.clone();
        self.data.hosts.extend(imported);
        if let Err(e) = self.persist() {
            self.data = old;
            return Err(e);
        }
        Ok(count)
    }
}
pub fn key(host: &str, port: u16) -> String {
    serde_json::to_string(&(host, port)).unwrap()
}
pub fn data_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenTermius")
        .join("opentermius-native.json")
}
pub fn open_default() -> Result<Store> {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    open_with_legacy_path(
        data_path(),
        base.join("LocalTerm").join("localterm-native.json"),
    )
}
fn open_with_legacy_path(path: PathBuf, legacy_path: PathBuf) -> Result<Store> {
    if path.exists() || !legacy_path.exists() {
        return Store::open(path);
    }
    let legacy = Store::open(legacy_path)?;
    let migrated = Store {
        path,
        data: legacy.data,
    };
    migrated.persist()?;
    Ok(migrated)
}
pub fn parse_quick_connect(value: &str) -> Result<Profile> {
    let mut tokens = shlex::split(value).context(crate::i18n::t("Invalid SSH command"))?;
    if tokens.first().is_some_and(|s| s == "ssh") {
        tokens.remove(0);
    }
    let mut host = None;
    let mut port = 22u16;
    let mut port_set = false;
    let mut username = String::new();
    let mut private_key_path = String::new();
    let mut key_requested = false;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "-p" | "-l" | "-i" => {
                ensure!(i + 1 < tokens.len(), "{}", crate::i18n::t("Missing SSH option value"));
                let option = &tokens[i];
                i += 1;
                match option.as_str() {
                    "-p" => {
                        port = parse_port(&tokens[i])?;
                        port_set = true;
                    }
                    "-l" => username = tokens[i].clone(),
                    _ => {
                        private_key_path = tokens[i].clone();
                        key_requested = true;
                    }
                }
            }
            v if !v.starts_with('-') && host.is_none() => host = Some(v.to_string()),
            _ => anyhow::bail!("{}", crate::i18n::t("Use ssh user@hostname -p port; remote commands are not supported here")),
        };
        i += 1;
    }
    let host = host.context(crate::i18n::t("Enter a host address"))?;
    let host = if let Some((user, host)) = host.rsplit_once('@') {
        username = user.into();
        host.to_string()
    } else {
        host
    };
    let (host, host_port) = split_host_port(&host)?;
    if !port_set {
        if let Some(host_port) = host_port {
            port = host_port;
        }
    }
    let mut profile = Profile {
        id: Uuid::new_v4().to_string(),
        name: String::new(),
        host,
        port,
        username,
        auth_type: if key_requested {
            "key".into()
        } else {
            "password".into()
        },
        private_key_path,
        key_id: String::new(),
        group: String::new(),
        encrypted_secret: None,
        tags: Vec::new(),
        os: None,
        last_connected: None,
        favorite: false,
        created_at: None,
    };
    profile.validate()?;
    Ok(profile)
}
fn parse_port(value: &str) -> Result<u16> {
    ensure!(!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()), "{}", crate::i18n::t("Invalid SSH port"));
    value.parse().context(crate::i18n::t("Invalid SSH port"))
}
/// Accept `host`, `host:port`, `[ipv6]` and `[ipv6]:port` without treating
/// any of the text as shell syntax.
fn split_host_port(value: &str) -> Result<(String, Option<u16>)> {
    if value.contains(['[', ']']) {
        let rest = value
            .strip_prefix('[')
            .context(crate::i18n::t("Invalid IPv6 address"))?;
        let close = rest.find(']').context(crate::i18n::t("Invalid IPv6 address"))?;
        let host = &rest[..close];
        ensure!(host.contains(':'), "{}", crate::i18n::t("Invalid IPv6 address"));
        let suffix = &rest[close + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            let port = suffix
                .strip_prefix(':')
                .context(crate::i18n::t("Invalid IPv6 address"))?;
            Some(parse_port(port)?)
        };
        Ok((host.to_string(), port))
    } else if value.matches(':').count() == 1 {
        let (host, port) = value.rsplit_once(':').expect("one colon");
        ensure!(!host.is_empty(), "{}", crate::i18n::t("Enter a host address"));
        Ok((host.to_string(), Some(parse_port(port)?)))
    } else {
        Ok((value.to_string(), None))
    }
}

#[cfg(windows)]
fn crypt(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    ensure!(input.len() <= u32::MAX as usize, "{}", crate::i18n::t("Credentials are too long"));
    let blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        let success = if encrypt {
            CryptProtectData(
                &blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        ensure!(success != 0, "{}", crate::tf!("Windows system encryption failed: {}", "0" => format!("{}", std::io::Error::last_os_error())));
        let result = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize).fill(0);
        LocalFree(out.pbData as *mut _);
        Ok(result)
    }
}
pub fn protect(secret: &str) -> Result<String> {
    ensure!(secret.len() <= 16384, "{}", crate::i18n::t("Credentials are too long"));
    #[cfg(windows)]
    {
        Ok(STANDARD.encode(crypt(secret.as_bytes(), true)?))
    }
    #[cfg(not(windows))]
    {
        secrets::protect(secret)
    }
}
pub fn unprotect(secret: &str) -> Result<String> {
    #[cfg(windows)]
    {
        String::from_utf8(crypt(&STANDARD.decode(secret)?, false)?).context(crate::i18n::t("Credential decryption failed"))
    }
    #[cfg(not(windows))]
    {
        secrets::unprotect(secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn new_configuration_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/data.json");
        Store::open(path.clone()).unwrap().persist().unwrap();
        assert_eq!(fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o077, 0);
    }
    #[test]
    fn product_rename_preserves_profiles_and_does_not_replace_current_data() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_path = temp.path().join("legacy/data.json");
        let current_path = temp.path().join("current/data.json");
        let mut legacy = Store::open(legacy_path.clone()).unwrap();
        legacy.save(profile(), None, false).unwrap();
        legacy.trust("localhost", 22, "SHA256:migration").unwrap();
        let migrated = open_with_legacy_path(current_path.clone(), legacy_path.clone()).unwrap();
        assert_eq!(migrated.data.hosts.len(), 1);
        assert_eq!(migrated.data.known_hosts, legacy.data.known_hosts);
        assert!(legacy_path.exists());
        let mut current = Store::open(current_path.clone()).unwrap();
        current.data.hosts.clear();
        current.persist().unwrap();
        assert!(
            open_with_legacy_path(current_path, legacy_path)
                .unwrap()
                .data
                .hosts
                .is_empty()
        );
    }
    #[test]
    fn quick_connect_is_parsed_without_shell_execution() {
        let p = parse_quick_connect("ssh tester@example.invalid -p 2222").unwrap();
        assert_eq!(
            (p.username.as_str(), p.host.as_str(), p.port),
            ("tester", "example.invalid", 2222)
        );
        assert!(parse_quick_connect("ssh host rm -rf /").is_err());
        assert!(parse_quick_connect("ssh -p 0 host").is_err());
        assert!(parse_quick_connect("[::1]").is_ok());
    }
    fn profile() -> Profile {
        Profile {
            id: String::new(),
            name: "测试".into(),
            host: "localhost".into(),
            port: 22,
            username: "tester".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            group: String::new(),
            encrypted_secret: None,
            tags: Vec::new(),
            os: None,
            last_connected: None,
            favorite: false,
            created_at: None,
        }
    }
    #[test]
    fn save_import_export_and_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(temp.path().join("data.json")).unwrap();
        s.save(profile(), Some("test-only-secret"), false).unwrap();
        let mut p = s.data.hosts[0].clone();
        p.port = 0;
        assert!(s.save(p, None, false).is_err());
        assert_eq!(s.data.hosts.len(), 1);
        let export = temp.path().join("export.json");
        s.export(&export).unwrap();
        s.import(&export).unwrap();
        assert_eq!(s.data.hosts.len(), 2);
        assert!(
            !fs::read_to_string(&s.path)
                .unwrap()
                .contains("test-only-secret")
        );
        assert_eq!(Store::open(s.path.clone()).unwrap().data.hosts.len(), 2);
    }
    #[test]
    fn trust_is_pinned() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(temp.path().join("data.json")).unwrap();
        s.trust("localhost", 22, "SHA256:first").unwrap();
        assert!(s.trust("localhost", 22, "SHA256:other").is_err());
    }
    #[test]
    fn known_host_keys_keep_their_serialized_format() {
        assert_eq!(key("example.com", 22), r#"["example.com",22]"#);
    }
    #[cfg(windows)]
    #[test]
    fn windows_dpapi_roundtrip() {
        let encrypted = protect("仅供测试的密码").unwrap();
        assert!(!encrypted.contains("密码"));
        assert_eq!(unprotect(&encrypted).unwrap(), "仅供测试的密码");
        assert!(unprotect("invalid").is_err());
    }
}

#[cfg(test)]
mod core_regression_tests {
    use super::*;
    use std::fs;

    fn test_profile(host: &str) -> Profile {
        Profile {
            id: String::new(),
            name: host.into(),
            host: host.into(),
            port: 22,
            username: "tester".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            group: String::new(),
            encrypted_secret: None,
            tags: Vec::new(),
            os: None,
            last_connected: None,
            favorite: false,
            created_at: None,
        }
    }

    fn valid_legacy(path: &Path) -> Store {
        let mut store = Store::open(path.to_path_buf()).unwrap();
        store.save(test_profile("legacy.invalid"), None, false).unwrap();
        store
    }

    #[test]
    fn migration_runs_only_when_the_current_file_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_path = temp.path().join("legacy.json");
        valid_legacy(&legacy_path);
        let current_path = temp.path().join("current.json");

        let mut current = fs::File::create(&current_path).unwrap();
        use std::io::Write;
        current.write_all(b"{").unwrap();
        drop(current);
        let before = fs::read(&current_path).unwrap();

        assert!(
            open_with_legacy_path(current_path.clone(), legacy_path.clone()).is_err(),
            "a corrupt current file must not fall back to legacy data"
        );
        assert_eq!(fs::read(&current_path).unwrap(), before);
        assert!(legacy_path.exists());
    }

    #[test]
    fn missing_current_and_legacy_files_produce_an_empty_store() {
        let temp = tempfile::tempdir().unwrap();
        let current_path = temp.path().join("current.json");
        let legacy_path = temp.path().join("legacy.json");
        let store = open_with_legacy_path(current_path.clone(), legacy_path.clone()).unwrap();
        assert!(store.data.hosts.is_empty());
        assert_eq!(store.data.version, 2);
        assert!(!current_path.exists());
        assert!(!legacy_path.exists());
    }

    #[test]
    fn corrupt_or_wrong_version_legacy_files_are_not_discarded_silently() {
        let temp = tempfile::tempdir().unwrap();
        let current_path = temp.path().join("current.json");

        let corrupt = temp.path().join("corrupt.json");
        fs::write(&corrupt, b"{ not json").unwrap();
        assert!(open_with_legacy_path(current_path.clone(), corrupt).is_err());
        assert!(!current_path.exists());

        let wrong = temp.path().join("wrong.json");
        fs::write(&wrong, br#"{"version":3,"hosts":[]}"#).unwrap();
        assert!(open_with_legacy_path(current_path.clone(), wrong).is_err());
        assert!(!current_path.exists());

        let invalid = temp.path().join("invalid.json");
        fs::write(
            &invalid,
            br#"{"version":2,"hosts":[{"name":"x","host":"bad host","username":"u"}]}"#,
        )
        .unwrap();
        assert!(open_with_legacy_path(current_path.clone(), invalid).is_err());
        assert!(!current_path.exists());
    }

    #[test]
    fn migration_persist_failure_keeps_the_legacy_file_readable() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_path = temp.path().join("legacy.json");
        valid_legacy(&legacy_path);

        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"not a directory").unwrap();
        let current_path = blocker.join("current.json");
        assert!(open_with_legacy_path(current_path.clone(), legacy_path.clone()).is_err());
        assert!(!current_path.exists());

        let reopened = Store::open(legacy_path).unwrap();
        assert_eq!(reopened.data.hosts.len(), 1);
        assert_eq!(reopened.data.hosts[0].host, "legacy.invalid");
    }

    #[cfg(windows)]
    #[test]
    fn migration_preserves_dpapi_encrypted_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_path = temp.path().join("legacy.json");
        let mut legacy = Store::open(legacy_path.clone()).unwrap();
        legacy
            .save(test_profile("legacy.invalid"), Some("migration-secret"), true)
            .unwrap();
        let encrypted = legacy.data.hosts[0]
            .encrypted_secret
            .clone()
            .expect("legacy secret");

        let current_path = temp.path().join("current").join("config.json");
        let migrated = open_with_legacy_path(current_path.clone(), legacy_path).unwrap();
        let saved = migrated.data.hosts[0]
            .encrypted_secret
            .as_deref()
            .expect("migrated secret");
        assert_eq!(saved, encrypted);
        assert_eq!(unprotect(saved).unwrap(), "migration-secret");
        assert!(current_path.exists());
    }
}
