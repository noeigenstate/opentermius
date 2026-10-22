//! Trusted fingerprints and conflict-preserving OpenSSH imports.
use crate::state::AppState;
use opentermius::{
    i18n::t,
    store::{self, Store},
};
use serde::Serialize;
use std::{collections::BTreeMap, io::Read, path::Path};

const MAX_IMPORT_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    host: String,
    port: u16,
    fingerprint: String,
    used_by: Vec<String>,
}

fn rows(store: &Store) -> Vec<KnownHost> {
    let mut usage = BTreeMap::<String, Vec<String>>::new();
    for profile in &store.data.hosts {
        usage
            .entry(store::key(&profile.host, profile.port))
            .or_default()
            .push(profile.name.clone());
    }
    let mut rows: Vec<_> = store
        .data
        .known_hosts
        .iter()
        .filter_map(|(key, fingerprint)| {
            let (host, port) = store::parse_key(key)?;
            let used_by = usage.get(key).cloned().unwrap_or_default();
            Some(KnownHost {
                host,
                port,
                fingerprint: fingerprint.clone(),
                used_by,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.host
            .to_lowercase()
            .cmp(&b.host.to_lowercase())
            .then(a.port.cmp(&b.port))
    });
    rows
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    added: usize,
    unchanged: usize,
    conflicts: usize,
    skipped: usize,
}

fn import_file(store: &mut Store, path: &Path) -> anyhow::Result<ImportSummary> {
    use anyhow::Context;
    let file = std::fs::File::open(path).context(t("Cannot read import file"))?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context(t("Cannot read import file"))?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_IMPORT_BYTES,
        "{}",
        t("Import file must not exceed 5 MB")
    );
    let parsed = store::parse_known_hosts(&String::from_utf8_lossy(&bytes));
    let result = store.import_known_hosts(&parsed.entries)?;
    Ok(ImportSummary {
        added: result.added,
        unchanged: result.unchanged,
        conflicts: result.conflicts,
        skipped: parsed.skipped,
    })
}

#[tauri::command]
pub fn list_known_hosts(state: tauri::State<'_, AppState>) -> Result<Vec<KnownHost>, String> {
    Ok(rows(&*state.store()?))
}

#[tauri::command]
pub fn delete_known_host(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
) -> Result<(), String> {
    state
        .store()?
        .forget(&host, port)
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn import_known_hosts(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<ImportSummary, String> {
    import_file(&mut *state.store()?, Path::new(&path)).map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_preserve_existing_fingerprints_and_report_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("config.json")).unwrap();
        let key =
            opentermius::keychain::generate("Fixture", opentermius::keychain::KeyKind::Ed25519)
                .unwrap();
        let path = dir.path().join("known_hosts");
        std::fs::write(&path, format!("fixture.invalid {}\n", key.public_key)).unwrap();
        assert_eq!(import_file(&mut store, &path).unwrap().added, 1);
        assert_eq!(import_file(&mut store, &path).unwrap().unchanged, 1);
        let other =
            opentermius::keychain::generate("Other", opentermius::keychain::KeyKind::Ed25519)
                .unwrap();
        std::fs::write(&path, format!("fixture.invalid {}\n", other.public_key)).unwrap();
        assert_eq!(import_file(&mut store, &path).unwrap().conflicts, 1);
        assert_eq!(rows(&store)[0].fingerprint, key.fingerprint);
        assert_eq!(
            Store::open(store.path.clone()).unwrap().data.known_hosts,
            store.data.known_hosts
        );
    }

    #[test]
    fn invalid_imports_are_counted_and_large_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("config.json")).unwrap();
        let path = dir.path().join("known_hosts");
        std::fs::write(&path, "# fixture\ninvalid-record\n").unwrap();
        let summary = import_file(&mut store, &path).unwrap();
        assert_eq!(summary.added, 0);
        assert_eq!(summary.skipped, 1);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        assert!(import_file(&mut store, &path).is_err());
        assert!(rows(&store).is_empty());
    }
}
