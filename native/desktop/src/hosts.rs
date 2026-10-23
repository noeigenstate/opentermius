//! Saved host commands. Persisted credentials never leave the backend.
use crate::state::AppState;
use opentermius::{
    hosts,
    i18n::t,
    store::{Profile, Store},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCard {
    id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    group: String,
    tags: Vec<String>,
    favorite: bool,
    last_connected: Option<String>,
}

/// Passwords are accepted only on save; reading a draft exposes just a presence flag.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDraft {
    id: String,
    name: String,
    host: String,
    port: String,
    username: String,
    group: String,
    tags: String,
    auth_type: String,
    private_key_path: String,
    key_id: String,
    #[serde(default, skip_serializing)]
    password: String,
    #[serde(default)]
    has_secret: bool,
}

impl From<&Profile> for HostDraft {
    fn from(profile: &Profile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            host: profile.host.clone(),
            port: profile.port.to_string(),
            username: profile.username.clone(),
            group: profile.group.clone(),
            tags: hosts::tags_string(profile),
            auth_type: profile.auth_type.clone(),
            private_key_path: profile.private_key_path.clone(),
            key_id: profile.key_id.clone(),
            password: String::new(),
            has_secret: profile.encrypted_secret.is_some(),
        }
    }
}

impl HostDraft {
    fn into_profile(self) -> Result<Profile, String> {
        let port = if self.port.trim().is_empty() {
            22
        } else {
            self.port
                .trim()
                .parse::<u16>()
                .map_err(|_| t("Port must be between 1 and 65535").to_string())?
        };
        let mut profile = Profile {
            id: self.id,
            name: self.name,
            host: self.host,
            port,
            username: self.username,
            group: self.group,
            tags: hosts::parse_tags(&self.tags),
            auth_type: self.auth_type,
            private_key_path: self.private_key_path,
            key_id: self.key_id,
            encrypted_secret: None,
            created_at: None,
            last_connected: None,
            favorite: false,
        };
        profile.validate().map_err(|error| error.to_string())?;
        Ok(profile)
    }
}

fn save_draft(store: &mut Store, mut draft: HostDraft) -> Result<String, String> {
    let password = Zeroizing::new(std::mem::take(&mut draft.password));
    if !draft.id.is_empty()
        && !store
            .data
            .hosts
            .iter()
            .any(|profile| profile.id == draft.id)
    {
        return Err(t("Host not found").to_string());
    }
    let mut profile = draft.into_profile()?;
    profile.favorite = store.data.hosts.iter().any(|saved| saved.id == profile.id && saved.favorite);
    let id = profile.id.clone();
    // Store preserves an existing secret on blank input and protects newly typed credentials.
    store
        .save(profile, Some(password.as_str()), true)
        .map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn list_hosts(state: tauri::State<'_, AppState>) -> Result<Vec<HostCard>, String> {
    Ok(state
        .store()?
        .data
        .hosts
        .iter()
        .map(|profile| HostCard {
            id: profile.id.clone(),
            name: profile.name.clone(),
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
            group: profile.group.clone(),
            tags: profile.tags.clone(),
            favorite: profile.favorite,
            last_connected: profile.last_connected.clone(),
        })
        .collect())
}

#[tauri::command]
pub fn get_host(state: tauri::State<'_, AppState>, id: String) -> Result<HostDraft, String> {
    state
        .store()?
        .data
        .hosts
        .iter()
        .find(|profile| profile.id == id)
        .map(HostDraft::from)
        .ok_or_else(|| t("Host not found").to_string())
}

#[tauri::command]
pub fn save_host(state: tauri::State<'_, AppState>, draft: HostDraft) -> Result<String, String> {
    save_draft(&mut *state.store()?, draft)
}

#[tauri::command]
pub fn delete_host(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .store()?
        .delete(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_host_favorite(state: tauri::State<'_, AppState>, id: String, favorite: bool) -> Result<(), String> {
    hosts::set_favorite(&mut *state.store()?, &id, favorite).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn duplicate_host(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    hosts::duplicate_profile(&mut *state.store()?, &id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn rename_group(
    state: tauri::State<'_, AppState>,
    old: String,
    new: String,
) -> Result<(), String> {
    hosts::rename_group(&mut *state.store()?, &old, &new)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeychainKey {
    id: String,
    label: String,
    kind: String,
}

#[tauri::command]
pub fn list_keychain_keys(state: tauri::State<'_, AppState>) -> Result<Vec<KeychainKey>, String> {
    Ok(state
        .store()?
        .data
        .keys
        .iter()
        .map(|key| KeychainKey {
            id: key.id.clone(),
            label: key.label.clone(),
            kind: key.key_type.clone(),
        })
        .collect())
}

#[tauri::command]
pub fn parse_quick_connect(text: String) -> Result<HostDraft, String> {
    let mut draft = opentermius::store::parse_quick_connect(&text)
        .map(|profile| HostDraft::from(&profile))
        .map_err(|error| error.to_string())?;
    // Parsing validates/generates a core id, but this draft has not been saved yet.
    draft.id.clear();
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> HostDraft {
        HostDraft {
            id: String::new(),
            name: "Example".into(),
            host: "example.invalid".into(),
            port: String::new(),
            username: String::new(),
            group: "Examples".into(),
            tags: " #demo, Demo, second ".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            password: String::new(),
            has_secret: false,
        }
    }

    #[test]
    fn profile_round_trip_preserves_editable_fields_and_hides_secrets() {
        let mut profile = draft().into_profile().unwrap();
        profile.port = 2222;
        profile.encrypted_secret = Some("opaque-test-ciphertext".into());
        let mut draft = HostDraft::from(&profile);
        assert!(draft.has_secret);
        assert!(draft.password.is_empty());
        draft.password = "test-only-input".into();
        let encoded = serde_json::to_string(&draft).unwrap();
        assert!(!encoded.contains("test-only-input"));
        assert!(!encoded.contains("opaque-test-ciphertext"));
        assert!(
            serde_json::from_str::<serde_json::Value>(&encoded)
                .unwrap()
                .get("password")
                .is_none()
        );
        let roundtrip = serde_json::from_str::<HostDraft>(&encoded)
            .unwrap()
            .into_profile()
            .unwrap();
        assert_eq!(roundtrip.id, profile.id);
        assert_eq!(roundtrip.name, profile.name);
        assert_eq!(roundtrip.host, profile.host);
        assert_eq!(roundtrip.port, profile.port);
        assert_eq!(roundtrip.username, profile.username);
        assert_eq!(roundtrip.group, profile.group);
        assert_eq!(roundtrip.tags, ["demo", "second"]);
        assert!(roundtrip.encrypted_secret.is_none());
    }

    #[test]
    fn conversion_uses_core_validation_and_port_default() {
        assert_eq!(draft().into_profile().unwrap().port, 22);
        for port in ["0", "65536", "-1", "invalid"] {
            let mut input = draft();
            input.port = port.into();
            assert!(input.into_profile().is_err());
        }
        let mut input = draft();
        input.host.clear();
        assert_eq!(
            input.into_profile().err().unwrap(),
            t("Host address is required")
        );
        let mut input = draft();
        input.auth_type = "key".into();
        assert!(input.into_profile().is_err());
    }

    #[test]
    fn key_drafts_support_keychain_and_files() {
        for (key_id, path) in [("example-key", ""), ("", "example-key-file")] {
            let mut input = draft();
            input.auth_type = "key".into();
            input.key_id = key_id.into();
            input.private_key_path = path.into();
            let profile = input.into_profile().unwrap();
            let output = HostDraft::from(&profile);
            assert_eq!(output.auth_type, "key");
            assert_eq!(output.key_id, key_id);
            assert_eq!(output.private_key_path, path);
        }
    }

    #[test]
    fn saves_encrypt_preserve_and_replace_credentials_and_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().join("hosts.json")).unwrap();
        let mut input = draft();
        input.password = "first-test-only-input".into();
        let id = save_draft(&mut store, input).unwrap();
        let encrypted = store.data.hosts[0].encrypted_secret.clone().unwrap();
        assert_ne!(encrypted, "first-test-only-input");
        assert_eq!(
            opentermius::store::unprotect(&encrypted).unwrap(),
            "first-test-only-input"
        );
        hosts::set_favorite(&mut store, &id, true).unwrap();
        let created = store.data.hosts[0].created_at.clone();
        hosts::mark_connected(&mut store, &id, "2024-01-01T00:00:00Z").unwrap();
        let mut edit = HostDraft::from(&store.data.hosts[0]);
        edit.name = "Edited".into();
        save_draft(&mut store, edit).unwrap();
        assert_eq!(
            store.data.hosts[0].encrypted_secret.as_ref(),
            Some(&encrypted)
        );
        assert_eq!(store.data.hosts[0].created_at, created);
        assert_eq!(
            store.data.hosts[0].last_connected.as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
        let mut edit = HostDraft::from(&store.data.hosts[0]);
        edit.password = "second-test-only-input".into();
        save_draft(&mut store, edit).unwrap();
        let reopened = Store::open(store.path.clone()).unwrap();
        assert!(reopened.data.hosts[0].favorite);
        assert_eq!(
            opentermius::store::unprotect(
                reopened.data.hosts[0].encrypted_secret.as_ref().unwrap()
            )
            .unwrap(),
            "second-test-only-input"
        );
        let mut edit = HostDraft::from(&store.data.hosts[0]);
        edit.auth_type = "key".into();
        edit.private_key_path = "example-key-file".into();
        save_draft(&mut store, edit).unwrap();
        assert!(store.data.hosts[0].encrypted_secret.is_none());
        let mut missing = draft();
        missing.id = "missing".into();
        assert!(save_draft(&mut store, missing).is_err());
        let mut invalid_key = HostDraft::from(&store.data.hosts[0]);
        invalid_key.key_id = "missing-key".into();
        assert!(save_draft(&mut store, invalid_key).is_err());
    }

    #[test]
    fn quick_connect_draft_is_unsaved_and_safe() {
        for text in ["ssh example.invalid:2222", "ssh -p 2222 example.invalid"] {
            let input = parse_quick_connect(text.into()).unwrap();
            assert!(input.id.is_empty());
            assert_eq!(input.host, "example.invalid");
            assert_eq!(input.port, "2222");
            assert!(!input.has_secret);
        }
        assert!(parse_quick_connect("ssh -p 0 example.invalid".into()).is_err());
    }
}
