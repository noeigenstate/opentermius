//! Keychain commands. DTOs deliberately contain only public key metadata.
use crate::state::AppState;
use opentermius::{
    i18n::t,
    keychain::{self, KeyEntry, KeyKind},
    store::Store,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    id: String,
    label: String,
    kind: String,
    fingerprint: String,
    public_key: String,
    created_at: String,
    used_by: Vec<String>,
}

fn key_info(entry: &KeyEntry, store: &Store) -> KeyInfo {
    KeyInfo {
        id: entry.id.clone(),
        label: entry.label.clone(),
        kind: entry.display_type().into(),
        fingerprint: entry.fingerprint.clone(),
        public_key: entry.public_key.clone(),
        created_at: entry.created_at.clone(),
        used_by: store
            .data
            .hosts
            .iter()
            .filter(|host| host.key_id == entry.id)
            .map(|host| host.name.clone())
            .collect(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenerateKind {
    Ed25519,
    Rsa3072,
    Rsa4096,
}

#[tauri::command]
pub fn list_keys(state: tauri::State<'_, AppState>) -> Result<Vec<KeyInfo>, String> {
    let store = state.store()?;
    Ok(store
        .data
        .keys
        .iter()
        .rev()
        .map(|entry| key_info(entry, &store))
        .collect())
}

#[tauri::command]
pub async fn generate_key(
    state: tauri::State<'_, AppState>,
    label: String,
    kind: GenerateKind,
) -> Result<(), String> {
    let kind = match kind {
        GenerateKind::Ed25519 => KeyKind::Ed25519,
        GenerateKind::Rsa3072 => KeyKind::Rsa3072,
        GenerateKind::Rsa4096 => KeyKind::Rsa4096,
    };
    let entry = tauri::async_runtime::spawn_blocking(move || keychain::generate(&label, kind))
        .await
        .map_err(|_| t("Cannot generate key").to_string())?
        .map_err(|error| format!("{error:#}"))?;
    state
        .store()?
        .add_key(entry)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn key_requires_passphrase(path: String) -> bool {
    keychain::file_requires_passphrase(Path::new(&path))
}

#[tauri::command]
pub async fn import_key(
    state: tauri::State<'_, AppState>,
    path: String,
    label: String,
    passphrase: Option<String>,
) -> Result<(), String> {
    let passphrase = passphrase.map(zeroize::Zeroizing::new);
    let entry = tauri::async_runtime::spawn_blocking(move || {
        keychain::import_file(
            Path::new(&path),
            &label,
            passphrase.as_deref().map(String::as_str),
        )
    })
    .await
    .map_err(|_| t("Cannot read private key").to_string())?
    .map_err(|error| format!("{error:#}"))?;
    state
        .store()?
        .add_key(entry)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn rename_key(
    state: tauri::State<'_, AppState>,
    id: String,
    label: String,
) -> Result<(), String> {
    state
        .store()?
        .rename_key(&id, &label)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn delete_key(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .store()?
        .delete_key(&id)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn export_public_key(
    state: tauri::State<'_, AppState>,
    id: String,
    path: String,
) -> Result<(), String> {
    let public_key = state
        .store()?
        .key_entry(&id)
        .ok_or_else(|| t("Keychain key not found").to_string())?
        .public_key
        .clone();
    std::fs::write(path, format!("{public_key}\n"))
        .map_err(|_| t("Cannot export public key").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_dto_never_serializes_encrypted_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("config.json")).unwrap();
        store.data.hosts.push(serde_json::from_value(serde_json::json!({"name": "Fixture", "host": "", "username": "", "keyId": "key-fixture"})).unwrap());
        let entry = KeyEntry {
            id: "key-fixture".into(),
            encrypted_private_key: "SECRET_SENTINEL".into(),
            encrypted_passphrase: Some("PASSPHRASE_SENTINEL".into()),
            ..Default::default()
        };
        let dto = key_info(&entry, &store);
        assert_eq!(dto.used_by, ["Fixture"]);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("SENTINEL"));
        assert!(!json.contains("encrypted"));
        assert!(serde_json::from_str::<GenerateKind>("\"unsupported\"").is_err());
    }
}
