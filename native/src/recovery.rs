//! Portable authenticated backups. Possessing a code without a passphrase grants
//! access to every credential it contains; the embedded key is intentional.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    num::NonZeroU32,
    path::{Path, PathBuf},
};
use zeroize::{Zeroize, Zeroizing};

use crate::store::{self, Data, Store};

const PREFIX: &str = "OTR1-";
const VERSION: u8 = 1;
const ITERATIONS: u32 = 600_000;
const MAX_PAYLOAD: usize = 12 * 1024 * 1024;
const MAX_CODE: usize = 20 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreMode {
    Merge,
    Replace,
}

#[derive(Default, Debug)]
pub struct RestoreSummary {
    pub hosts: usize,
    pub keys: usize,
    pub snippets: usize,
    pub key_files: usize,
    pub known_hosts: usize,
    pub skipped: usize,
    pub fingerprint_conflicts: Vec<String>,
}
impl std::fmt::Display for RestoreSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::tf!(
            "Restored {} hosts, {} keys, {} snippets, {} key files, {} trust records; skipped {} existing items and {} fingerprint conflicts (kept local fingerprints)",
            "0" => self.hosts.to_string(),
            "1" => self.keys.to_string(),
            "2" => self.snippets.to_string(),
            "3" => self.key_files.to_string(),
            "4" => self.known_hosts.to_string(),
            "5" => self.skipped.to_string(),
            "6" => self.fingerprint_conflicts.len().to_string()
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    data: Value,
    key_files: BTreeMap<String, String>,
    // JSON pointers distinguish a protected empty secret from an absent string
    // credential. The portable data itself still contains ordinary plaintext.
    #[serde(default)]
    empty_secrets: Vec<String>,
}
fn wipe(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(items) => items.values_mut().for_each(wipe),
        _ => {}
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        wipe(&mut self.data);
        self.key_files.values_mut().for_each(Zeroize::zeroize);
        self.empty_secrets.iter_mut().for_each(Zeroize::zeroize);
    }
}

/// Transform serialized credentials, including future nested objects/arrays.
pub fn transform_encrypted_fields(
    value: &mut Value,
    transform: &mut impl FnMut(&str) -> Result<String>,
) -> Result<()> {
    transform_encrypted_at(value, "", &mut |_, secret| transform(secret))
}

fn transform_encrypted_at(
    value: &mut Value,
    path: &str,
    transform: &mut impl FnMut(&str, &str) -> Result<String>,
) -> Result<()> {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
                if name.starts_with("encrypted") {
                    if let Value::String(secret) = value {
                        let replacement = transform(&path, secret)?;
                        secret.zeroize();
                        *secret = replacement;
                        continue;
                    }
                }
                transform_encrypted_at(value, &path, transform)?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                transform_encrypted_at(value, &format!("{path}/{index}"), transform)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_ids(data: &Value) -> Result<()> {
    for section in ["hosts", "keys", "snippets"] {
        let rows = data[section].as_array().context(crate::i18n::t("Recovery data is missing a list"))?;
        ensure!(rows.len() <= 10_000, "{}", crate::i18n::t("Too many recovery data items"));
        let mut seen = HashSet::new();
        for row in rows {
            let id = row["id"].as_str().context(crate::i18n::t("Recovery data is missing an ID"))?;
            ensure!(!id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{}", crate::i18n::t("Invalid recovery data ID"));
            ensure!(seen.insert(id), "{}", crate::i18n::t("Recovery data contains duplicate IDs"));
        }
    }
    Ok(())
}

pub fn generate(data: &Data, passphrase: Option<&str>, include_secrets: bool) -> Result<String> {
    ensure!(data.version == 2, "{}", crate::i18n::t("Unsupported configuration version"));
    let mut payload = Payload {
        data: serde_json::to_value(data)?,
        key_files: BTreeMap::new(),
        empty_secrets: Vec::new(),
    };
    validate_ids(&payload.data)?;
    let mut empty_secrets = Vec::new();
    transform_encrypted_at(&mut payload.data, "", &mut |path, value| {
        if !include_secrets || value.is_empty() {
            return Ok(String::new());
        }
        let plaintext = store::unprotect(value)?;
        if plaintext.is_empty() {
            empty_secrets.push(path.to_owned());
        }
        Ok(plaintext)
    })?;
    payload.empty_secrets = empty_secrets;
    if include_secrets {
        let mut embedded_size = 0;
        for host in &data.hosts {
            if !host.private_key_path.is_empty() {
                let path = Path::new(&host.private_key_path);
                ensure!(fs::metadata(path)
                        .context(crate::i18n::t("Cannot read private key file; no incomplete backup was generated"))?
                        .len()
                        <= 1024 * 1024, "{}", crate::i18n::t("Private key file is too large"));
                let bytes =
                    Zeroizing::new(fs::read(path).context(crate::i18n::t("Cannot read private key file; no incomplete backup was generated"))?);
                embedded_size += bytes.len();
                ensure!(embedded_size <= MAX_PAYLOAD * 3 / 4, "{}", crate::i18n::t("Total private key file size is too large"));
                payload
                    .key_files
                    .insert(host.id.clone(), URL_SAFE_NO_PAD.encode(&*bytes));
            }
        }
    }
    let serialized = Zeroizing::new(serde_json::to_vec(&payload)?);
    ensure!(serialized.len() <= MAX_PAYLOAD, "{}", crate::i18n::t("Recovery data is too large"));
    let mut compressed = Zeroizing::new(miniz_oxide::deflate::compress_to_vec_zlib(&serialized, 6));
    let random = SystemRandom::new();
    let passphrase = passphrase.filter(|s| !s.is_empty());
    // Header: version, mode, compression, salt(16) or embedded key(32), nonce(12).
    // Authenticate the whole header as AAD, including key derivation mode.
    let mut header = vec![VERSION, u8::from(passphrase.is_some()), 1];
    let mut key = Zeroizing::new([0u8; 32]);
    if let Some(passphrase) = passphrase {
        let mut salt = [0u8; 16];
        random
            .fill(&mut salt)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Cannot generate secure random bytes")))?;
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(ITERATIONS).unwrap(),
            &salt,
            passphrase.as_bytes(),
            &mut *key,
        );
        header.extend_from_slice(&salt);
    } else {
        random
            .fill(&mut *key)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Cannot generate secure random bytes")))?;
        header.extend_from_slice(&*key);
    }
    let mut nonce = [0u8; 12];
    random
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Cannot generate secure random bytes")))?;
    header.extend_from_slice(&nonce);
    let cipher = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &*key)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Recovery encryption initialization failed")))?,
    );
    cipher
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(&header),
            &mut *compressed,
        )
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Recovery data encryption failed")))?;
    let mut output = Zeroizing::new(header);
    output.extend_from_slice(&compressed);
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(&*output)))
}

fn decode(code: &str, passphrase: Option<&str>) -> Result<Payload> {
    ensure!(code.len() <= MAX_CODE, "{}", crate::i18n::t("Recovery code is too long"));
    let compact = Zeroizing::new(
        code.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>(),
    );
    let encoded = compact
        .strip_prefix(PREFIX)
        .context(crate::i18n::t("Invalid recovery code prefix or version"))?;
    let mut bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).context(crate::i18n::t("Invalid recovery code encoding"))?);
    ensure!(bytes.len() >= 3, "{}", crate::i18n::t("Recovery code is incomplete"));
    ensure!(bytes[0] == VERSION, "{}", crate::i18n::t("Unsupported recovery code version"));
    ensure!(bytes[1] <= 1 && bytes[2] == 1, "{}", crate::i18n::t("Unsupported recovery code format"));
    let passphrase_mode = bytes[1] == 1;
    let header_len = 3 + if passphrase_mode { 16 } else { 32 } + 12;
    ensure!(bytes.len() >= header_len + aead::AES_256_GCM.tag_len(), "{}", crate::i18n::t("Recovery code is incomplete"));
    let mut key = Zeroizing::new([0u8; 32]);
    if passphrase_mode {
        let passphrase = passphrase
            .filter(|s| !s.is_empty())
            .context(crate::i18n::t("This recovery code requires a password; enter it and try again"))?;
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(ITERATIONS).unwrap(),
            &bytes[3..19],
            passphrase.as_bytes(),
            &mut *key,
        );
    } else {
        key.copy_from_slice(&bytes[3..35]);
    }
    let nonce: [u8; 12] = bytes[header_len - 12..header_len].try_into()?;
    let cipher = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &*key)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Recovery decryption initialization failed")))?,
    );
    let (header, ciphertext) = bytes.split_at_mut(header_len);
    let compressed = cipher
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(header),
            ciphertext,
        )
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Recovery code is corrupted or the password is incorrect")))?;
    let json = Zeroizing::new(
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(compressed, MAX_PAYLOAD)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Recovery data compression is invalid or too large")))?,
    );
    let payload: Payload = serde_json::from_slice(&json)
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Invalid recovery data format")))?;
    ensure!(payload.data["version"] == 2, "{}", crate::i18n::t("Unsupported configuration version"));
    validate_ids(&payload.data)?;
    Ok(payload)
}

fn merge_rows(
    current: &mut Value,
    incoming: &Value,
    mode: RestoreMode,
    skipped: &mut usize,
) -> Result<usize> {
    let rows = current.as_array_mut().context(crate::i18n::t("Invalid local configuration list"))?;
    let mut count = 0;
    for row in incoming.as_array().context(crate::i18n::t("Invalid recovery data list"))? {
        if let Some(existing) = rows.iter_mut().find(|r| r["id"] == row["id"]) {
            if mode == RestoreMode::Merge {
                *skipped += 1;
                continue;
            }
            *existing = row.clone();
        } else {
            rows.push(row.clone());
        }
        count += 1;
    }
    Ok(count)
}

fn merge_preferences(current: &mut Value, incoming: &Value, mode: RestoreMode, fresh: bool) {
    if mode == RestoreMode::Replace || fresh {
        *current = incoming.clone();
    }
}

// Roll back newly created files on any error, including config persistence.
#[derive(Default)]
struct CreatedFiles {
    paths: Vec<PathBuf>,
    committed: bool,
}
impl Drop for CreatedFiles {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn write_key(
    directory: &Path,
    id: &str,
    bytes: &[u8],
    created: &mut CreatedFiles,
) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    ensure!(!fs::symlink_metadata(directory)?.file_type().is_symlink(), "{}", crate::i18n::t("Key directory must not be a symbolic link"));
    let mut path = directory.join(format!("{id}.key"));
    let mut file = loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                path = directory.join(format!("{id}-{}.key", uuid::Uuid::new_v4()));
            }
            Err(e) => return Err(e).context(crate::i18n::t("Cannot write restored private key")),
        }
    };
    created.paths.push(path.clone());
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(path)
}

pub fn restore(
    store: &mut Store,
    code: &str,
    passphrase: Option<&str>,
    mode: RestoreMode,
) -> Result<RestoreSummary> {
    let mut payload = decode(code, passphrase)?;
    transform_encrypted_fields(&mut payload.data, &mut |value| {
        if value.is_empty() {
            Ok(String::new())
        } else {
            store::protect(value)
        }
    })?;
    for path in &payload.empty_secrets {
        let field = path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .replace("~1", "/")
            .replace("~0", "~");
        ensure!(field.starts_with("encrypted"), "{}", crate::i18n::t("Invalid recovery code credential location"));
        let value = payload
            .data
            .pointer_mut(path)
            .context(crate::i18n::t("Invalid recovery code credential location"))?;
        ensure!(value.as_str() == Some(""), "{}", crate::i18n::t("Invalid recovery code empty credential marker"));
        *value = Value::String(store::protect("")?);
    }
    let mut incoming: Data = serde_json::from_value(payload.data.clone())
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Invalid restored configuration")))?;
    for host in &mut incoming.hosts {
        if host.encrypted_secret.as_deref() == Some("") {
            host.encrypted_secret = None;
        }
        host.validate()?;
    }
    for id in payload.key_files.keys() {
        ensure!(incoming.hosts.iter().any(|host| &host.id == id), "{}", crate::i18n::t("Private key file has no associated host"));
    }
    let mut summary = RestoreSummary::default();
    let mut files = CreatedFiles::default();
    let directory = store.path.parent().context(crate::i18n::t("Invalid configuration directory"))?.join("keys");
    for host in &mut incoming.hosts {
        if mode == RestoreMode::Merge && store.data.hosts.iter().any(|h| h.id == host.id) {
            continue;
        }
        if let Some(encoded) = payload.key_files.get(&host.id) {
            let bytes = Zeroizing::new(
                URL_SAFE_NO_PAD
                    .decode(encoded)
                    .context(crate::i18n::t("Invalid private key file encoding"))?,
            );
            ensure!(bytes.len() <= 1024 * 1024, "{}", crate::i18n::t("Private key file is too large"));
            host.private_key_path = write_key(&directory, &host.id, &bytes, &mut files)?
                .to_string_lossy()
                .into_owned();
            summary.key_files += 1;
        }
    }
    let mut merged = serde_json::to_value(&store.data)?;
    let incoming = serde_json::to_value(&incoming)?;
    summary.hosts = merge_rows(
        &mut merged["hosts"],
        &incoming["hosts"],
        mode,
        &mut summary.skipped,
    )?;
    summary.keys = merge_rows(
        &mut merged["keys"],
        &incoming["keys"],
        mode,
        &mut summary.skipped,
    )?;
    summary.snippets = merge_rows(
        &mut merged["snippets"],
        &incoming["snippets"],
        mode,
        &mut summary.skipped,
    )?;
    // A new installation may already have persisted its empty default config.
    // Treat that as fresh too, so the one-click default restores preferences.
    let fresh = store.data.hosts.is_empty()
        && store.data.keys.is_empty()
        && store.data.snippets.is_empty()
        && store.data.known_hosts.is_empty()
        && merged["settings"] == serde_json::to_value(crate::settings::Settings::default())?
        && merged["hostsView"] == serde_json::to_value(crate::hosts::HostsView::default())?;
    merge_preferences(&mut merged["settings"], &incoming["settings"], mode, fresh);
    merge_preferences(&mut merged["hostsView"], &incoming["hostsView"], mode, fresh);
    let known = merged["knownHosts"]
        .as_object_mut()
        .context(crate::i18n::t("Invalid local fingerprint list"))?;
    for (host, fingerprint) in incoming["knownHosts"]
        .as_object()
        .context(crate::i18n::t("Invalid recovery fingerprint list"))?
    {
        match known.get(host) {
            Some(existing) if existing != fingerprint => {
                summary.fingerprint_conflicts.push(host.clone())
            }
            Some(_) => {}
            None => {
                known.insert(host.clone(), fingerprint.clone());
                summary.known_hosts += 1;
            }
        }
    }
    let updated: Data = serde_json::from_value(merged)?;
    ensure!(serde_json::to_vec_pretty(&updated)?.len() < 16 * 1024 * 1024, "{}", crate::i18n::t("Merged configuration is too large"));
    let previous = std::mem::replace(&mut store.data, updated);
    if let Err(error) = store.persist() {
        store.data = previous;
        bail!("{}", crate::tf!("Failed to save restored configuration: {error}", "error" => format!("{}", error)));
    }
    files.committed = true;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_secret_locations_use_escaped_json_pointers_through_arrays() {
        let mut value = json!({"nested": [{"encrypted/a~": "cipher", "encryptedMissing": ""}]});
        let mut paths = Vec::new();
        transform_encrypted_at(&mut value, "", &mut |path, secret| {
            if secret == "cipher" {
                paths.push(path.to_owned());
            }
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(paths, ["/nested/0/encrypted~1a~0"]);
        assert_eq!(value.pointer(&paths[0]).unwrap(), "");
        assert_eq!(value["nested"][0]["encryptedMissing"], "");
    }

    #[test]
    fn preferences_merge_preserves_local_replace_restores_all_and_fresh_merge_restores() {
        // Use serialized values so this test covers future preferences even while
        // the parallel settings feature is still scaffolded.
        let local = json!({"fontSize": 14, "terminal": {"theme": "local"}});
        let backup = json!({"fontSize": 18, "terminal": {"theme": "backup"}, "future": true});
        for (mode, fresh, expected) in [
            (RestoreMode::Merge, false, &local),
            (RestoreMode::Replace, false, &backup),
            (RestoreMode::Merge, true, &backup),
        ] {
            let mut current = local.clone();
            merge_preferences(&mut current, &backup, mode, fresh);
            assert_eq!(&current, expected);
        }
    }
}
