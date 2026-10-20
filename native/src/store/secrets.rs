//! Versioned portable secrets: AES-256-GCM with a fresh random nonce per value.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

const PREFIX: &str = "v1:";
const AAD: &[u8] = b"OpenTermius secrets v1";
const NONCE_LEN: usize = 12;

fn random_key() -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0; 32]);
    SystemRandom::new()
        .fill(key.as_mut())
        .map_err(|_| anyhow::anyhow!("系统随机数不可用"))?;
    Ok(key)
}

fn cipher(key: &[u8; 32]) -> Result<aead::LessSafeKey> {
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| anyhow::anyhow!("凭据密钥无效"))?,
    ))
}

fn encrypt(key: &[u8; 32], secret: &str) -> Result<String> {
    ensure!(secret.len() <= 16384, "凭据过长");
    let mut nonce = [0; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("系统随机数不可用"))?;
    let mut payload = Zeroizing::new(secret.as_bytes().to_vec());
    cipher(key)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut *payload,
        )
        .map_err(|_| anyhow::anyhow!("凭据加密失败"))?;
    let mut encoded = nonce.to_vec();
    encoded.extend_from_slice(&payload);
    Ok(format!("{PREFIX}{}", STANDARD.encode(encoded)))
}

fn decrypt(key: &[u8; 32], secret: &str) -> Result<String> {
    ensure!(secret.len() <= 22000, "凭据过长");
    let encoded = secret
        .strip_prefix(PREFIX)
        .context("凭据加密格式不受支持（Windows DPAPI 凭据不能跨平台解密）")?;
    let mut payload = Zeroizing::new(STANDARD.decode(encoded).context("凭据编码无效")?);
    ensure!(
        payload.len() >= NONCE_LEN + aead::AES_256_GCM.tag_len(),
        "凭据已损坏"
    );
    let nonce: [u8; NONCE_LEN] = payload[..NONCE_LEN].try_into()?;
    let plain = cipher(key)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut payload[NONCE_LEN..],
        )
        .map_err(|_| anyhow::anyhow!("凭据解密失败或已被修改"))?;
    Ok(std::str::from_utf8(plain)
        .context("凭据不是有效 UTF-8")?
        .to_owned())
}

#[cfg(unix)]
static DIRECTORY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Set once at application startup to keep --data-dir credentials isolated.
#[cfg(unix)]
pub fn configure_directory(path: std::path::PathBuf) -> Result<()> {
    DIRECTORY
        .set(path)
        .map_err(|_| anyhow::anyhow!("凭据目录已设置"))
}

#[cfg(unix)]
fn directory() -> std::path::PathBuf {
    DIRECTORY
        .get()
        .cloned()
        .unwrap_or_else(|| super::data_path().parent().unwrap().to_owned())
}

#[cfg(not(windows))]
pub(super) fn protect(secret: &str) -> Result<String> {
    let key = master_key(true)?;
    encrypt(&key, secret)
}

#[cfg(not(windows))]
pub(super) fn unprotect(secret: &str) -> Result<String> {
    // Reject foreign/unknown formats before accessing the platform key store.
    ensure!(secret.starts_with(PREFIX), "凭据加密格式不受支持");
    let key = master_key(false)?;
    decrypt(&key, secret)
}

#[cfg(target_os = "linux")]
fn master_key(create: bool) -> Result<Zeroizing<[u8; 32]>> {
    file_key::load(&directory(), create)
}

#[cfg(target_os = "macos")]
fn master_key(create: bool) -> Result<Zeroizing<[u8; 32]>> {
    use security_framework::os::macos::keychain::SecKeychain;
    const SERVICE: &str = "OpenTermius AES master key v1";
    let keychain = SecKeychain::default().context("无法打开 macOS 钥匙串")?;
    // Separate isolated configurations without storing their path in Keychain.
    let path = directory();
    if create {
        std::fs::create_dir_all(&path)?;
    }
    let path = path.canonicalize().context("凭据目录不存在")?;
    let account = STANDARD.encode(ring::digest::digest(
        &ring::digest::SHA256,
        path.as_os_str().as_encoded_bytes(),
    ));
    match keychain.find_generic_password(SERVICE, &account) {
        Ok((password, _)) => {
            ensure!(password.len() == 32, "钥匙串主密钥已损坏");
            let mut key = Zeroizing::new([0; 32]);
            key.copy_from_slice(&password);
            Ok(key)
        }
        Err(error) if error.code() == -25300 && create => {
            let key = random_key()?;
            // Add-only: never replace a key created by another process.
            match keychain.add_generic_password(SERVICE, &account, key.as_ref()) {
                Ok(()) => Ok(key),
                Err(error) if error.code() == -25299 => master_key(false),
                Err(error) => Err(error).context("无法保存钥匙串主密钥"),
            }
        }
        Err(error) => Err(error).context("无法读取钥匙串主密钥"),
    }
}

#[cfg(any(target_os = "linux", all(test, unix)))]
#[path = "secrets/file_key.rs"]
mod file_key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_aes_roundtrip_uses_unique_nonces() {
        let key = random_key().unwrap();
        for plain in ["", "test-password", "密码🔑"] {
            let first = encrypt(&key, plain).unwrap();
            let second = encrypt(&key, plain).unwrap();
            assert!(first.starts_with(PREFIX));
            assert_ne!(first, second);
            assert_eq!(decrypt(&key, &first).unwrap(), plain);
        }
    }

    #[test]
    fn portable_aes_rejects_wrong_keys_tampering_and_bad_formats() {
        let key = random_key().unwrap();
        let value = encrypt(&key, "secret").unwrap();
        assert!(decrypt(&random_key().unwrap(), &value).is_err());
        let original = STANDARD
            .decode(value.strip_prefix(PREFIX).unwrap())
            .unwrap();
        for index in [0, NONCE_LEN, original.len() - 1] {
            let mut damaged = original.clone();
            damaged[index] ^= 1;
            assert!(decrypt(&key, &format!("{PREFIX}{}", STANDARD.encode(damaged))).is_err());
        }
        for invalid in ["", "v2:AAAA", "DPAPI-base64", "v1:!", "v1:AAAA"] {
            assert!(decrypt(&key, invalid).is_err());
        }
        assert!(encrypt(&key, &"x".repeat(16385)).is_err());
        assert!(decrypt(&key, &"x".repeat(22001)).is_err());
    }
}
