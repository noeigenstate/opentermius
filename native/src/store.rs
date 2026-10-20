use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

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
            (&mut self.name, "名称", 128),
            (&mut self.host, "地址", 253),
            (&mut self.username, "用户名", 128),
            (&mut self.group, "分组", 128),
        ] {
            *value = value.trim().into();
            ensure!(
                value.chars().count() <= max
                    && value.len() <= max * 4
                    && !value.chars().any(char::is_control),
                "{label}无效"
            );
        }
        ensure!(!self.host.is_empty(), "Host address is required");
        ensure!(
            !self.host.chars().any(char::is_whitespace),
            "地址无效"
        );
        if self.name.is_empty() {
            self.name = self.host.chars().take(128).collect();
        }
        ensure!(self.port > 0, "端口必须在 1–65535 之间");
        ensure!(
            self.auth_type == "password" || self.auth_type == "key",
            "认证方式无效"
        );
        self.private_key_path = self.private_key_path.trim().into();
        ensure!(
            self.private_key_path.chars().count() <= 32767
                && self.private_key_path.len() <= 32767 * 4
                && !self.private_key_path.chars().any(char::is_control),
            "私钥路径无效"
        );
        ensure!(
            self.auth_type != "key"
                || !self.private_key_path.is_empty()
                || !self.key_id.is_empty(),
            "请选择私钥文件或 Keychain 密钥"
        );
        if self.id.is_empty() {
            self.id = Uuid::new_v4().to_string();
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
    pub forwards: Vec<crate::forwarding::Forward>,
    #[serde(default)]
    pub snippets: Vec<crate::snippets::Snippet>,
    #[serde(default)]
    pub settings: crate::settings::Settings,
}
pub struct Store {
    pub path: PathBuf,
    pub data: Data,
}
impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            ensure!(
                fs::metadata(&path)?.len() < 16 * 1024 * 1024,
                "本机配置文件过大"
            );
            let mut data: Data = serde_json::from_slice(&fs::read(&path)?)
                .context("本机配置损坏，请保留文件并从备份恢复")?;
            ensure!(data.version == 2, "本机配置版本不受支持");
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
        let parent = self.path.parent().context("无效的数据目录")?;
        let encoded = serde_json::to_vec_pretty(&self.data)?;
        ensure!(encoded.len() < 16 * 1024 * 1024, "本机配置文件过大");
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
        ensure!(
            profile.key_id.is_empty()
                || profile.auth_type != "key"
                || self.data.keys.iter().any(|key| key.id == profile.key_id),
            "所选 Keychain 密钥不存在"
        );
        let old = self.data.clone();
        let previous = self.data.hosts.iter().find(|p| p.id == profile.id);
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
        ensure!(
            !host.trim().is_empty()
                && !host.chars().any(|c| c.is_whitespace() || c.is_control())
                && host.len() <= 253 * 4,
            "主机地址无效"
        );
        ensure!(
            !fingerprint.trim().is_empty()
                && !fingerprint.chars().any(|c| c.is_whitespace() || c.is_control())
                && fingerprint.len() <= 512,
            "主机指纹无效"
        );
        let key = key(host, port);
        if let Some(previous) = self.data.known_hosts.get(&key) {
            ensure!(previous == fingerprint, "SSH 主机密钥已变化，拒绝连接");
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
    /// Merges imported OpenSSH entries without replacing existing pins.
    pub fn import_known_hosts(
        &mut self,
        entries: &[KnownHostEntry],
    ) -> Result<KnownHostsMerge> {
        let previous = self.data.known_hosts.clone();
        let outcome = merge_known_hosts(&mut self.data.known_hosts, entries);
        if outcome.added > 0 {
            if let Err(error) = self.persist() {
                self.data.known_hosts = previous;
                return Err(error);
            }
        }
        Ok(outcome)
    }
    /// Removes a pinned fingerprint so the next connection asks for confirmation.
    pub fn forget(&mut self, host: &str, port: u16) -> Result<bool> {
        let previous = self.data.known_hosts.clone();
        if !remove_known_host(&mut self.data.known_hosts, host, port) {
            return Ok(false);
        }
        if let Err(error) = self.persist() {
            self.data.known_hosts = previous;
            return Err(error);
        }
        Ok(true)
    }
    pub fn export(&self, path: &Path) -> Result<()> {
        ensure!(path != self.path, "不能导出到当前配置文件");
        if path.exists() && self.path.exists() {
            ensure!(
                fs::canonicalize(path)? != fs::canonicalize(&self.path)?,
                "不能导出到当前配置文件"
            );
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
        ensure!(
            fs::metadata(path)?.len() < 16 * 1024 * 1024,
            "导入文件不能超过 16 MB"
        );
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let hosts = value
            .get("hosts")
            .and_then(|v| v.as_array())
            .context("缺少主机列表")?;
        ensure!(hosts.len() <= 10000, "主机数量过多");
        let mut imported = Vec::new();
        for item in hosts {
            let mut p: Profile = serde_json::from_value(item.clone())?;
            p.id = Uuid::new_v4().to_string();
            p.encrypted_secret = None;
            p.validate()?;
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
/// Inverse of [`key`]; `None` for a malformed or unusable stored key.
pub fn parse_key(value: &str) -> Option<(String, u16)> {
    let (host, port) = serde_json::from_str::<(String, u16)>(value).ok()?;
    if host.is_empty() || port == 0 {
        return None;
    }
    Some((host, port))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedKnownHosts {
    pub entries: Vec<KnownHostEntry>,
    pub skipped: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KnownHostsMerge {
    pub added: usize,
    pub unchanged: usize,
    pub conflicts: usize,
}
/// Computes the same `SHA256:…` fingerprint that [`Store::trust`] stores.
pub fn openssh_fingerprint(key_type: &str, key_base64: &str) -> Option<String> {
    let key = russh::keys::PublicKey::from_openssh(&format!("{key_type} {key_base64}")).ok()?;
    Some(key.fingerprint(russh::keys::HashAlg::Sha256).to_string())
}
/// Parses plain OpenSSH `known_hosts` lines. Comments, hashed entries and
/// marker lines are ignored; malformed or unusable lines are counted as skipped.
pub fn parse_known_hosts(input: &str) -> ParsedKnownHosts {
    let mut parsed = ParsedKnownHosts::default();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('@') {
            parsed.skipped += 1;
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(host_field) = fields.next() else {
            continue;
        };
        let (Some(key_type), Some(key_base64)) = (fields.next(), fields.next()) else {
            parsed.skipped += 1;
            continue;
        };
        if host_field
            .split(',')
            .any(|host| host.trim().starts_with("|1|"))
        {
            parsed.skipped += 1;
            continue;
        }
        let hosts = host_field
            .split(',')
            .map(parse_known_host_token)
            .collect::<Option<Vec<_>>>();
        let Some(hosts) = hosts.filter(|hosts| !hosts.is_empty()) else {
            parsed.skipped += 1;
            continue;
        };
        let Some(fingerprint) = openssh_fingerprint(key_type, key_base64) else {
            parsed.skipped += 1;
            continue;
        };
        for (host, port) in hosts {
            let entry = KnownHostEntry {
                host,
                port,
                fingerprint: fingerprint.clone(),
            };
            if !parsed.entries.contains(&entry) {
                parsed.entries.push(entry);
            }
        }
    }
    parsed
}
fn parse_known_host_token(token: &str) -> Option<(String, u16)> {
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
        return None;
    }
    let (host, port) = if let Some(rest) = token.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        if host.is_empty() || host.contains(['[', ']']) {
            return None;
        }
        (host, port.parse::<u16>().ok()?)
    } else {
        if token.contains(['[', ']']) {
            return None;
        }
        (token, 22)
    };
    if host.is_empty()
        || port == 0
        || host.chars().any(char::is_whitespace)
        || host.contains(['*', '?'])
        || host.starts_with('!')
    {
        return None;
    }
    Some((host.to_string(), port))
}
/// Merges entries into `existing` without overwriting a different fingerprint.
pub fn merge_known_hosts(
    existing: &mut BTreeMap<String, String>,
    entries: &[KnownHostEntry],
) -> KnownHostsMerge {
    let mut outcome = KnownHostsMerge::default();
    for entry in entries {
        let key = key(&entry.host, entry.port);
        match existing.get(&key) {
            Some(current) if current == &entry.fingerprint => outcome.unchanged += 1,
            Some(_) => outcome.conflicts += 1,
            None => {
                existing.insert(key, entry.fingerprint.clone());
                outcome.added += 1;
            }
        }
    }
    outcome
}
pub fn remove_known_host(
    existing: &mut BTreeMap<String, String>,
    host: &str,
    port: u16,
) -> bool {
    existing.remove(&key(host, port)).is_some()
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
    let mut tokens = shlex::split(value).context("Invalid SSH command")?;
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
                ensure!(i + 1 < tokens.len(), "Missing SSH option value");
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
            _ => anyhow::bail!(
                "Use ssh user@hostname -p port; remote commands are not supported here"
            ),
        };
        i += 1;
    }
    let host = host.context("Enter a host address")?;
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
    };
    profile.validate()?;
    Ok(profile)
}
fn parse_port(value: &str) -> Result<u16> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "Invalid SSH port"
    );
    value.parse().context("Invalid SSH port")
}
/// Accept `host`, `host:port`, `[ipv6]` and `[ipv6]:port` without treating
/// any of the text as shell syntax.
fn split_host_port(value: &str) -> Result<(String, Option<u16>)> {
    if value.contains(['[', ']']) {
        let rest = value
            .strip_prefix('[')
            .context("IPv6 地址格式无效")?;
        let close = rest.find(']').context("IPv6 地址格式无效")?;
        let host = &rest[..close];
        ensure!(host.contains(':'), "IPv6 地址格式无效");
        let suffix = &rest[close + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            let port = suffix
                .strip_prefix(':')
                .context("IPv6 地址格式无效")?;
            Some(parse_port(port)?)
        };
        Ok((host.to_string(), port))
    } else if value.matches(':').count() == 1 {
        let (host, port) = value.rsplit_once(':').expect("one colon");
        ensure!(!host.is_empty(), "Enter a host address");
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
    ensure!(input.len() <= u32::MAX as usize, "凭据过长");
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
        ensure!(
            success != 0,
            "Windows 系统加密操作失败：{}",
            std::io::Error::last_os_error()
        );
        let result = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize).fill(0);
        LocalFree(out.pbData as *mut _);
        Ok(result)
    }
}
pub fn protect(secret: &str) -> Result<String> {
    ensure!(secret.len() <= 16384, "凭据过长");
    #[cfg(windows)]
    {
        Ok(STANDARD.encode(crypt(secret.as_bytes(), true)?))
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("此版本仅支持 Windows 系统加密")
    }
}
pub fn unprotect(secret: &str) -> Result<String> {
    #[cfg(windows)]
    {
        String::from_utf8(crypt(&STANDARD.decode(secret)?, false)?).context("凭据解密失败")
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("此版本仅支持 Windows 系统加密")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    const ED25519_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XFSqti";
    const ED25519_FINGERPRINT: &str = "SHA256:UCUiLr7Pjs9wFFJMDByLgc3NrtdU344OgUM45wZPcIQ";
    #[test]
    fn known_host_keys_are_formatted_and_parsed() {
        assert_eq!(key("example.com", 22), r#"["example.com",22]"#);
        assert_eq!(
            parse_key(&key("example.com", 2222)),
            Some(("example.com".into(), 2222))
        );
        assert_eq!(
            parse_key(&key("2001:db8::1", 2200)),
            Some(("2001:db8::1".into(), 2200))
        );
        assert_eq!(parse_key("example.com"), None);
        assert_eq!(parse_key(r#"["",22]"#), None);
        assert_eq!(parse_key(r#"["example.com",0]"#), None);
    }
    #[test]
    fn openssh_fingerprints_match_russh_sha256_format() {
        assert_eq!(
            openssh_fingerprint("ssh-ed25519", ED25519_KEY).as_deref(),
            Some(ED25519_FINGERPRINT)
        );
        assert_eq!(openssh_fingerprint("ssh-ed25519", "not-base64"), None);
        assert_eq!(openssh_fingerprint("ssh-ed25519", "AAAA"), None);
    }
    #[test]
    fn openssh_parser_handles_ports_hosts_and_skips_unusable_lines() {
        let input = format!(
            "# comment\n\
             \n\
                # indented comment\n\
             example.com ssh-ed25519 {ED25519_KEY} one\n\
             [example.com]:2222 ssh-ed25519 {ED25519_KEY} two\n\
             [2001:db8::1]:2200 ssh-ed25519 {ED25519_KEY} three\n\
             a.example,b.example ssh-ed25519 {ED25519_KEY} four\n\
             |1|Zm9vYmFy|YmF6 ssh-ed25519 {ED25519_KEY} five\n\
             @revoked * ssh-ed25519 {ED25519_KEY} six\n\
             @cert-authority *.example.com ssh-ed25519 {ED25519_KEY} seven\n\
             bad.example ssh-ed25519 not-base64 eight\n\
             missing.example ssh-ed25519\n\
             [bad.example]:0 ssh-ed25519 {ED25519_KEY} nine\n\
             *.example.com ssh-ed25519 {ED25519_KEY} ten\n"
        );
        let parsed = parse_known_hosts(&input);
        assert_eq!(parsed.skipped, 7);
        assert_eq!(parsed.entries.len(), 5);
        for (host, port) in [
            ("example.com", 22),
            ("example.com", 2222),
            ("2001:db8::1", 2200),
            ("a.example", 22),
            ("b.example", 22),
        ] {
            assert!(
                parsed.entries.contains(&KnownHostEntry {
                    host: host.into(),
                    port,
                    fingerprint: ED25519_FINGERPRINT.into(),
                }),
                "missing {host}:{port}"
            );
        }
    }
    #[test]
    fn merge_keeps_existing_fingerprints_and_counts_conflicts() {
        let mut known = BTreeMap::new();
        known.insert(key("same.example", 22), "SHA256:existing".to_string());
        let entries = vec![
            KnownHostEntry {
                host: "new.example".into(),
                port: 22,
                fingerprint: "SHA256:new".into(),
            },
            KnownHostEntry {
                host: "same.example".into(),
                port: 22,
                fingerprint: "SHA256:existing".into(),
            },
            KnownHostEntry {
                host: "same.example".into(),
                port: 22,
                fingerprint: "SHA256:different".into(),
            },
            KnownHostEntry {
                host: "2001:db8::1".into(),
                port: 2222,
                fingerprint: "SHA256:v6".into(),
            },
        ];
        let outcome = merge_known_hosts(&mut known, &entries);
        assert_eq!(
            outcome,
            KnownHostsMerge {
                added: 2,
                unchanged: 1,
                conflicts: 1,
            }
        );
        assert_eq!(
            known.get(&key("same.example", 22)).map(String::as_str),
            Some("SHA256:existing")
        );
        assert_eq!(
            known.get(&key("new.example", 22)).map(String::as_str),
            Some("SHA256:new")
        );
        assert_eq!(
            known.get(&key("2001:db8::1", 2222)).map(String::as_str),
            Some("SHA256:v6")
        );
    }
    #[test]
    fn import_and_forget_persist_without_overwriting_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.json");
        let mut store = Store::open(path.clone()).unwrap();
        let entries = vec![KnownHostEntry {
            host: "example.com".into(),
            port: 22,
            fingerprint: "SHA256:first".into(),
        }];
        assert_eq!(
            store.import_known_hosts(&entries).unwrap(),
            KnownHostsMerge {
                added: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            Store::open(path.clone()).unwrap().data.known_hosts.len(),
            1
        );
        let conflict = vec![KnownHostEntry {
            host: "example.com".into(),
            port: 22,
            fingerprint: "SHA256:second".into(),
        }];
        assert_eq!(
            store.import_known_hosts(&conflict).unwrap(),
            KnownHostsMerge {
                conflicts: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            store
                .data
                .known_hosts
                .get(&key("example.com", 22))
                .map(String::as_str),
            Some("SHA256:first")
        );
        assert!(store.forget("example.com", 22).unwrap());
        assert!(store.data.known_hosts.is_empty());
        assert!(!store.forget("example.com", 22).unwrap());
        assert!(
            Store::open(path)
                .unwrap()
                .data
                .known_hosts
                .is_empty()
        );
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
