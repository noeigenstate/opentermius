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
                value.len() <= max * 4 && !value.chars().any(char::is_control),
                "{label}无效"
            );
        }
        ensure!(!self.host.is_empty(), "Host address is required");
        if self.name.is_empty() {
            self.name = self.host.clone();
        }
        ensure!(self.port > 0, "端口必须在 1–65535 之间");
        ensure!(
            self.auth_type == "password" || self.auth_type == "key",
            "认证方式无效"
        );
        ensure!(
            self.auth_type != "key" || !self.private_key_path.is_empty(),
            "请选择私钥文件"
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
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!("opentermius-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(&self.data)?)?;
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
    pub fn export(&self, path: &Path) -> Result<()> {
        let mut data = self.data.clone();
        data.known_hosts.clear();
        for p in &mut data.hosts {
            p.encrypted_secret = None;
        }
        fs::write(path, serde_json::to_vec_pretty(&data)?)?;
        Ok(())
    }
    pub fn import(&mut self, path: &Path) -> Result<usize> {
        ensure!(
            fs::metadata(path)?.len() < 5 * 1024 * 1024,
            "导入文件不能超过 5 MB"
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
    let mut username = String::new();
    let mut private_key_path = String::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "-p" | "-l" | "-i" => {
                ensure!(i + 1 < tokens.len(), "Missing SSH option value");
                let option = &tokens[i];
                i += 1;
                match option.as_str() {
                    "-p" => port = tokens[i].parse().context("Invalid SSH port")?,
                    "-l" => username = tokens[i].clone(),
                    _ => private_key_path = tokens[i].clone(),
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
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let mut profile = Profile {
        id: Uuid::new_v4().to_string(),
        name: host.clone(),
        host,
        port,
        username,
        auth_type: if private_key_path.is_empty() {
            "password".into()
        } else {
            "key".into()
        },
        private_key_path,
        group: String::new(),
        encrypted_secret: None,
    };
    profile.validate()?;
    Ok(profile)
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
    #[cfg(windows)]
    #[test]
    fn windows_dpapi_roundtrip() {
        let encrypted = protect("仅供测试的密码").unwrap();
        assert!(!encrypted.contains("密码"));
        assert_eq!(unprotect(&encrypted).unwrap(), "仅供测试的密码");
        assert!(unprotect("invalid").is_err());
    }
}
