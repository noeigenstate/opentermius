//! SSH key pairs stored in the local keychain.
//!
//! Private key material is kept as an OpenSSH PEM string encrypted with Windows
//! DPAPI (`store::protect`). Plaintext exists only while a key is generated,
//! imported, exported or used to authenticate.
use crate::store::{Store, protect, unprotect};
use anyhow::{Context, Result, bail, ensure};
use russh::keys::{
    HashAlg, PrivateKey,
    key::safe_rng,
    ssh_key::{
        self, Algorithm, LineEnding,
        private::{KeypairData, RsaKeypair},
    },
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use zeroize::Zeroizing;

/// Imported key files larger than this are rejected instead of being parsed.
pub const MAX_KEY_FILE: u64 = 1024 * 1024;
const MAX_KEYS: usize = 500;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// `ed25519`, `rsa` or `ecdsa`.
    #[serde(default)]
    pub key_type: String,
    /// OpenSSH one-line public key, including its comment.
    #[serde(default)]
    pub public_key: String,
    /// OpenSSH SHA-256 fingerprint (`SHA256:...`).
    #[serde(default)]
    pub fingerprint: String,
    /// DPAPI-protected OpenSSH PEM private key. Never log this value.
    #[serde(default)]
    pub encrypted_private_key: String,
    /// DPAPI-protected passphrase supplied when the key was imported.
    #[serde(default)]
    pub encrypted_passphrase: Option<String>,
    /// UTC creation timestamp (RFC 3339, second precision).
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyKind {
    #[default]
    Ed25519,
    Rsa3072,
    Rsa4096,
}

impl KeyKind {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Rsa3072,
            2 => Self::Rsa4096,
            _ => Self::Ed25519,
        }
    }

    pub const fn index(self) -> i32 {
        match self {
            Self::Ed25519 => 0,
            Self::Rsa3072 => 1,
            Self::Rsa4096 => 2,
        }
    }

    pub const fn key_type(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            _ => "rsa",
        }
    }

    pub const fn default_label(self) -> &'static str {
        match self {
            Self::Ed25519 => "Ed25519 key",
            Self::Rsa3072 => "RSA 3072 key",
            Self::Rsa4096 => "RSA 4096 key",
        }
    }

    pub const fn rsa_bits(self) -> Option<usize> {
        match self {
            Self::Ed25519 => None,
            Self::Rsa3072 => Some(3072),
            Self::Rsa4096 => Some(4096),
        }
    }
}

/// Decrypted key material handed to the SSH transport.
///
/// The `Debug` implementation intentionally omits every secret.
#[derive(Clone, Default)]
pub struct KeyMaterial {
    pub pem: Zeroizing<String>,
    pub passphrase: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyMaterial")
            .field("has_passphrase", &self.passphrase.is_some())
            .finish_non_exhaustive()
    }
}

impl KeyMaterial {
    /// Parse the stored OpenSSH PEM into a usable private key. Runs in a
    /// blocking context because RSA parsing is expensive.
    pub fn into_private_key(self) -> Result<PrivateKey> {
        decode_private_key(
            self.pem.as_str(),
            self.passphrase.as_deref().map(|p| p.as_str()),
        )
    }
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn clean_label(label: &str) -> Result<String> {
    let label = label.trim();
    ensure!(!label.is_empty(), "请输入密钥名称");
    ensure!(
        label.chars().count() <= 128 && !label.chars().any(char::is_control),
        "密钥名称无效"
    );
    Ok(label.to_string())
}

/// Map an `ssh-key` algorithm to the stable keychain type string.
fn key_type_of(key: &PrivateKey) -> Result<&'static str> {
    match key.algorithm() {
        Algorithm::Ed25519 => Ok("ed25519"),
        Algorithm::Rsa { .. } => Ok("rsa"),
        Algorithm::Ecdsa { .. } => Ok("ecdsa"),
        _ => bail!("暂不支持该类型的 SSH 私钥"),
    }
}

/// Public key line, type and fingerprint derived from a private key.
fn describe(key: &PrivateKey) -> Result<(&'static str, String, String)> {
    let public_key = key.public_key().to_openssh().context("无法导出公钥")?;
    ensure!(
        !public_key.is_empty()
            && public_key.lines().count() == 1
            && !public_key.chars().any(char::is_control),
        "公钥格式无效"
    );
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    ensure!(
        fingerprint.starts_with("SHA256:") && fingerprint.len() > "SHA256:".len(),
        "指纹格式无效"
    );
    Ok((key_type_of(key)?, public_key, fingerprint))
}

fn entry_from_key(label: &str, key: &PrivateKey, passphrase: Option<&str>) -> Result<KeyEntry> {
    ensure!(!key.is_encrypted(), "私钥仍受口令保护");
    let label = clean_label(label)?;
    let (key_type, public_key, fingerprint) = describe(key)?;
    let pem = key.to_openssh(LineEnding::LF)?;
    let encrypted_private_key = protect(&pem)?;
    let encrypted_passphrase = passphrase
        .filter(|value| !value.is_empty())
        .map(protect)
        .transpose()?;
    Ok(KeyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        label,
        key_type: key_type.into(),
        public_key,
        fingerprint,
        encrypted_private_key,
        encrypted_passphrase,
        created_at: now(),
    })
}

/// Generate a fresh key pair. RSA keys can take seconds, so callers should run
/// this off the UI thread.
pub fn generate(label: &str, kind: KeyKind) -> Result<KeyEntry> {
    let label = if label.trim().is_empty() {
        kind.default_label().to_string()
    } else {
        clean_label(label)?
    };
    let mut rng = safe_rng();
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
        KeyKind::Rsa3072 => {
            let pair = RsaKeypair::random(&mut rng, 3072)?;
            PrivateKey::new(KeypairData::from(pair), label.as_str())?
        }
        KeyKind::Rsa4096 => {
            let pair = RsaKeypair::random(&mut rng, 4096)?;
            PrivateKey::new(KeypairData::from(pair), label.as_str())?
        }
    };
    key.set_comment(label.as_str());
    entry_from_key(&label, &key, None)
}

/// Decode OpenSSH, PKCS#1 and PKCS#8 private keys, using the passphrase when
/// the key is encrypted.
pub fn decode_private_key(text: &str, passphrase: Option<&str>) -> Result<PrivateKey> {
    // PPK parser errors can contain entire input lines. Do not retain them as
    // an error source: callers display both Display and Debug error chains.
    let key = russh::keys::decode_secret_key(text, passphrase)
        .map_err(|_| anyhow::anyhow!("私钥或口令无效"))?;
    ensure!(!key.is_encrypted(), "私钥口令无效或缺失");
    Ok(key)
}

/// Import a private key from memory. The decrypted OpenSSH PEM is re-encrypted
/// with DPAPI; the supplied passphrase is also kept (DPAPI-protected) so an
/// exported copy can retain its original protection.
pub fn import_private_key(label: &str, text: &str, passphrase: Option<&str>) -> Result<KeyEntry> {
    let label = if label.trim().is_empty() {
        "Imported key"
    } else {
        label
    };
    let key = decode_private_key(text, passphrase)?;
    entry_from_key(label, &key, passphrase)
}

pub fn import_file(path: &Path, label: &str, passphrase: Option<&str>) -> Result<KeyEntry> {
    ensure!(
        fs::metadata(path).context("读取私钥失败")?.len() <= MAX_KEY_FILE,
        "私钥不能超过 1 MB"
    );
    let text = Zeroizing::new(fs::read_to_string(path).context("读取私钥失败")?);
    let fallback = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported key");
    import_private_key(
        if label.trim().is_empty() {
            fallback
        } else {
            label
        },
        &text,
        passphrase,
    )
}

/// Whether a key file appears to be passphrase protected, so the UI can ask
/// for the passphrase instead of reporting a decode error.
pub fn text_requires_passphrase(text: &str) -> bool {
    let text = text.trim();
    if text.starts_with("PuTTY-User-Key-File-") {
        return text.contains("Encryption: aes") || text.contains("Encryption: chacha");
    }
    if text.contains("BEGIN ENCRYPTED PRIVATE KEY") || text.contains("Proc-Type: 4,ENCRYPTED") {
        return true;
    }
    match ssh_key::PrivateKey::from_openssh(text.as_bytes()) {
        Ok(key) => key.is_encrypted(),
        Err(_) => false,
    }
}

pub fn file_requires_passphrase(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() > MAX_KEY_FILE {
        return false;
    }
    fs::read_to_string(path)
        .map(|text| text_requires_passphrase(&Zeroizing::new(text)))
        .unwrap_or(false)
}

impl KeyEntry {
    pub fn validate(&mut self) -> Result<()> {
        self.label = clean_label(&self.label)?;
        ensure!(
            matches!(self.key_type.as_str(), "ed25519" | "rsa" | "ecdsa"),
            "密钥类型无效"
        );
        ensure!(
            self.public_key.starts_with("ssh-") || self.public_key.starts_with("ecdsa-"),
            "公钥格式无效"
        );
        ensure!(
            self.public_key.lines().count() == 1 && !self.public_key.chars().any(char::is_control),
            "公钥格式无效"
        );
        ensure!(self.fingerprint.starts_with("SHA256:"), "指纹格式无效");
        ensure!(!self.encrypted_private_key.is_empty(), "密钥材料缺失");
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        }
        if self.created_at.is_empty() {
            self.created_at = now();
        }
        Ok(())
    }

    pub fn display_type(&self) -> &'static str {
        match self.key_type.as_str() {
            "ed25519" => "Ed25519",
            "rsa" => "RSA",
            "ecdsa" => "ECDSA",
            _ => "SSH",
        }
    }

    pub fn display_created(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|_| self.created_at.clone())
    }

    /// Decrypt this entry into material suitable for the SSH transport.
    pub fn load(&self) -> Result<KeyMaterial> {
        let pem = Zeroizing::new(unprotect(&self.encrypted_private_key).context("无法解密私钥")?);
        let passphrase = self
            .encrypted_passphrase
            .as_deref()
            .map(unprotect)
            .transpose()
            .context("无法解密私钥口令")?
            .map(Zeroizing::new);
        Ok(KeyMaterial { pem, passphrase })
    }

    pub fn private_key(&self) -> Result<PrivateKey> {
        self.load()?.into_private_key()
    }

    /// Serialize the private key for an explicit export. A stored passphrase is
    /// applied again so the exported file keeps the protection it had on
    /// import.
    pub fn export_private(&self) -> Result<Zeroizing<String>> {
        let material = self.load()?;
        let passphrase = material.passphrase.clone();
        let key = material.into_private_key()?;
        match passphrase.as_deref() {
            Some(passphrase) if !passphrase.is_empty() => {
                let encrypted = key.encrypt(&mut safe_rng(), passphrase.as_str())?;
                Ok(encrypted.to_openssh(LineEnding::LF)?)
            }
            _ => Ok(key.to_openssh(LineEnding::LF)?),
        }
    }
}

impl Store {
    pub fn add_key(&mut self, mut entry: KeyEntry) -> Result<()> {
        entry.validate()?;
        ensure!(
            !self
                .data
                .keys
                .iter()
                .any(|key| key.fingerprint == entry.fingerprint),
            "该密钥已存在"
        );
        ensure!(self.data.keys.len() < MAX_KEYS, "密钥数量过多");
        let old = self.data.clone();
        self.data.keys.push(entry);
        if let Err(error) = self.persist() {
            self.data = old;
            return Err(error);
        }
        Ok(())
    }

    pub fn rename_key(&mut self, id: &str, label: &str) -> Result<()> {
        let label = clean_label(label)?;
        let old = self.data.clone();
        let entry = self
            .data
            .keys
            .iter_mut()
            .find(|key| key.id == id)
            .context("Keychain 密钥不存在")?;
        entry.label = label;
        if let Err(error) = self.persist() {
            self.data = old;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_key(&mut self, id: &str) -> Result<()> {
        let entry = self
            .data
            .keys
            .iter()
            .find(|key| key.id == id)
            .context("Keychain 密钥不存在")?;
        let hosts: Vec<String> = self
            .data
            .hosts
            .iter()
            .filter(|host| host.key_id == entry.id)
            .map(|host| format!("“{}”", host.name))
            .collect();
        ensure!(
            hosts.is_empty(),
            "无法删除：主机 {} 正在使用该密钥",
            hosts.join("、")
        );
        let old = self.data.clone();
        self.data.keys.retain(|key| key.id != id);
        if let Err(error) = self.persist() {
            self.data = old;
            return Err(error);
        }
        Ok(())
    }

    pub fn key_entry(&self, id: &str) -> Option<&KeyEntry> {
        self.data.keys.iter().find(|key| key.id == id)
    }

    pub fn key_material(&self, id: &str) -> Result<KeyMaterial> {
        self.key_entry(id).context("Keychain 密钥不存在")?.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Profile;

    #[cfg(windows)]
    #[test]
    fn generated_key_is_encrypted_and_roundtrips_through_json() {
        let entry = generate("工作密钥", KeyKind::Ed25519).unwrap();
        assert_eq!(entry.key_type, "ed25519");
        assert!(entry.public_key.starts_with("ssh-ed25519 "));
        assert!(entry.public_key.contains("工作密钥"));
        assert!(entry.fingerprint.starts_with("SHA256:"));
        assert!(!entry.encrypted_private_key.contains("PRIVATE KEY"));
        let rest = entry.fingerprint.strip_prefix("SHA256:").unwrap();
        assert_eq!(rest.len(), 43);
        assert!(
            rest.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
        );
        assert!(!rest.ends_with('='));

        let json = serde_json::to_value(&entry).unwrap();
        assert!(json.get("encryptedPrivateKey").is_some());
        assert!(json.get("encryptedPassphrase").is_some());
        assert!(
            !json["encryptedPrivateKey"]
                .as_str()
                .unwrap()
                .contains("PRIVATE KEY")
        );
        let parsed: KeyEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, entry);
        let key = parsed.private_key().unwrap();
        assert_eq!(key.public_key().to_openssh().unwrap(), entry.public_key);
        assert_eq!(
            key.fingerprint(HashAlg::Sha256).to_string(),
            entry.fingerprint
        );
    }

    #[cfg(windows)]
    #[test]
    fn rsa_key_sizes_are_supported() {
        assert_eq!(KeyKind::Rsa3072.rsa_bits(), Some(3072));
        assert_eq!(KeyKind::Rsa4096.rsa_bits(), Some(4096));
        assert_eq!(KeyKind::from_index(1), KeyKind::Rsa3072);
        assert_eq!(KeyKind::from_index(2), KeyKind::Rsa4096);
        for (kind, bits) in [(KeyKind::Rsa3072, 3072), (KeyKind::Rsa4096, 4096)] {
            let entry = generate("rsa", kind).unwrap();
            assert_eq!(entry.key_type, "rsa");
            assert!(entry.public_key.starts_with("ssh-rsa "));
            let parsed: KeyEntry =
                serde_json::from_str(&serde_json::to_string(&entry).unwrap()).unwrap();
            let key = parsed.private_key().unwrap();
            assert_eq!(key.key_data().rsa().unwrap().key_size(), bits);
            assert_eq!(
                key.fingerprint(HashAlg::Sha256).to_string(),
                entry.fingerprint
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn imported_key_from_a_temp_file_matches_its_fingerprint() {
        let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let expected = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let pem = key.to_openssh(LineEnding::LF).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("id_test");
        fs::write(&path, pem.as_bytes()).unwrap();
        assert!(!file_requires_passphrase(&path));
        let entry = import_file(&path, "imported", None).unwrap();
        assert_eq!(entry.label, "imported");
        assert_eq!(entry.fingerprint, expected);
        assert_eq!(entry.public_key, key.public_key().to_openssh().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn passphrase_protected_import_requires_the_right_passphrase() {
        let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let encrypted = key.encrypt(&mut safe_rng(), "correct horse").unwrap();
        let pem = encrypted.to_openssh(LineEnding::LF).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("id_protected");
        fs::write(&path, pem.as_bytes()).unwrap();
        assert!(file_requires_passphrase(&path));
        assert!(import_file(&path, "protected", None).is_err());
        assert!(import_file(&path, "protected", Some("wrong")).is_err());
        let entry = import_file(&path, "protected", Some("correct horse")).unwrap();
        assert!(entry.encrypted_passphrase.is_some());
        assert_eq!(
            entry
                .private_key()
                .unwrap()
                .public_key()
                .to_openssh()
                .unwrap(),
            key.public_key().to_openssh().unwrap()
        );
        let exported = entry.export_private().unwrap();
        assert!(decode_private_key(&exported, None).is_err());
        assert!(decode_private_key(&exported, Some("correct horse")).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn pem_and_ecdsa_imports_preserve_the_public_key() {
        for algorithm in [
            Algorithm::Ed25519,
            Algorithm::Ecdsa {
                curve: ssh_key::EcdsaCurve::NistP256,
            },
        ] {
            let key = PrivateKey::random(&mut safe_rng(), algorithm.clone()).unwrap();
            let mut pem = Zeroizing::new(Vec::new());
            russh::keys::encode_pkcs8_pem(&key, &mut *pem).unwrap();
            let entry =
                import_private_key("PEM import", std::str::from_utf8(&pem).unwrap(), None).unwrap();
            assert_eq!(entry.public_key, key.public_key().to_openssh().unwrap());
            assert_eq!(entry.private_key().unwrap().algorithm(), algorithm);

            let openssh = key.to_openssh(LineEnding::LF).unwrap();
            let entry = import_private_key("OpenSSH import", &openssh, None).unwrap();
            assert_eq!(entry.key_type, key_type_of(&key).unwrap());
            assert_eq!(entry.public_key, key.public_key().to_openssh().unwrap());
        }
    }

    #[cfg(windows)]
    #[test]
    fn delete_is_refused_while_a_host_references_the_key() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().join("data.json")).unwrap();
        let entry = generate("开发密钥", KeyKind::Ed25519).unwrap();
        let id = entry.id.clone();
        store.add_key(entry).unwrap();
        store.save(profile(Some(id.clone())), None, false).unwrap();
        let error = store.delete_key(&id).unwrap_err().to_string();
        assert!(error.contains("无法删除"));
        assert!(error.contains("开发机"));
        assert!(store.key_entry(&id).is_some());

        store.rename_key(&id, "新名称").unwrap();
        assert_eq!(store.key_entry(&id).unwrap().label, "新名称");
        assert_eq!(
            Store::open(store.path.clone())
                .unwrap()
                .key_entry(&id)
                .unwrap()
                .label,
            "新名称"
        );

        let host = store.data.hosts[0].id.clone();
        store.delete(&host).unwrap();
        store.delete_key(&id).unwrap();
        assert!(store.data.keys.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn host_save_rejects_a_missing_keychain_key() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().join("data.json")).unwrap();
        let error = store
            .save(profile(Some("missing-key".into())), None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("密钥不存在"));
        assert!(store.data.hosts.is_empty());
    }

    #[test]
    fn existing_host_profiles_default_to_file_or_password_auth() {
        let mut value = serde_json::to_value(profile(None)).unwrap();
        value.as_object_mut().unwrap().remove("keyId");
        let mut parsed: Profile = serde_json::from_value(value.clone()).unwrap();
        assert!(parsed.key_id.is_empty());
        parsed.validate().unwrap();
        value["authType"] = "key".into();
        value["privateKeyPath"] = "existing-key.pem".into();
        let mut parsed: Profile = serde_json::from_value(value).unwrap();
        assert!(parsed.key_id.is_empty());
        parsed.validate().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn host_export_omits_keychain_material_and_saved_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().join("data.json")).unwrap();
        let entry = generate("export test", KeyKind::Ed25519).unwrap();
        let id = entry.id.clone();
        store.add_key(entry.clone()).unwrap();
        store.save(profile(Some(id)), None, false).unwrap();
        store
            .save(profile(None), Some("host-password"), true)
            .unwrap();
        store.trust("localhost", 22, "SHA256:test").unwrap();
        let path = temp.path().join("hosts.json");
        store.export(&path).unwrap();
        let text = fs::read_to_string(path).unwrap();
        let exported: crate::store::Data = serde_json::from_str(&text).unwrap();
        assert!(exported.keys.is_empty());
        assert!(exported.known_hosts.is_empty());
        assert!(
            exported
                .hosts
                .iter()
                .all(|host| host.encrypted_secret.is_none())
        );
        assert!(!text.contains(&entry.encrypted_private_key));
        assert!(!text.contains("host-password"));
        assert_eq!(store.data.keys.len(), 1);
        assert!(store.data.hosts[1].encrypted_secret.is_some());
    }

    fn profile(key_id: Option<String>) -> Profile {
        let key_id = key_id.unwrap_or_default();
        Profile {
            id: String::new(),
            name: "开发机".into(),
            host: "localhost".into(),
            port: 22,
            username: "tester".into(),
            auth_type: if key_id.is_empty() {
                "password".into()
            } else {
                "key".into()
            },
            private_key_path: String::new(),
            key_id,
            group: String::new(),
            tags: Vec::new(),
            last_connected: None,
            created_at: None,
            encrypted_secret: None,
        }
    }
}
