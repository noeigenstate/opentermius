//! Connection history shares a single LogBook with the session lifecycle.
use crate::state::AppState;
use opentermius::logs::LogEntry;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    #[serde(flatten)]
    entry: LogEntry,
    can_connect: bool,
}

fn row(mut entry: LogEntry, host_ids: &[String]) -> HistoryRow {
    let can_connect =
        entry.kind == "ssh" && !entry.host_id.is_empty() && host_ids.contains(&entry.host_id);
    if let Some(error) = &mut entry.error {
        *error = opentermius::i18n::translate(error).to_string();
    }
    HistoryRow { entry, can_connect }
}

#[tauri::command]
pub fn list_logs(state: tauri::State<'_, AppState>) -> Result<Vec<HistoryRow>, String> {
    let host_ids = state
        .store()?
        .data
        .hosts
        .iter()
        .map(|host| host.id.clone())
        .collect::<Vec<_>>();
    let book = state
        .logs
        .lock()
        .map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())?;
    Ok(book
        .as_ref()
        .map_err(Clone::clone)?
        .recent(opentermius::logs::MAX_ENTRIES)
        .into_iter()
        .map(|entry| row(entry, &host_ids))
        .collect())
}

#[tauri::command]
pub fn clear_logs(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut book = state
        .logs
        .lock()
        .map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())?;
    book.as_mut().map_err(|error| error.clone())?.clear()
        .map_err(|error| opentermius::tf!("Cannot read or write connection history: {error}", "error" => format!("{error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_is_only_available_for_existing_ssh_hosts() {
        let mut entry = LogEntry {
            id: "fixture".into(),
            host_id: "saved".into(),
            label: "Fixture".into(),
            address: "Fixture".into(),
            kind: "ssh".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            ended_at: None,
            status: "connecting".into(),
            error: None,
        };
        assert!(row(entry.clone(), &["saved".into()]).can_connect);
        assert!(!row(entry.clone(), &[]).can_connect);
        entry.kind = "local".into();
        assert!(!row(entry, &["saved".into()]).can_connect);
    }

    /// Manual smoke window with disposable data. Set OPENTERMIUS_PAGE_SMOKE_URL
    /// to this worktree's Vite URL, then close the window to end the check.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn pages_live_smoke() {
        use opentermius::{keychain, logs, snippets, store::Store};
        use std::sync::Mutex;

        let Ok(url) = std::env::var("OPENTERMIUS_PAGE_SMOKE_URL") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("config.json")).unwrap();
        let key = keychain::generate("Fixture key", keychain::KeyKind::Ed25519).unwrap();
        store.data.hosts.push(
            serde_json::from_value(serde_json::json!({
                "id": "fixture-host", "name": "Fixture host", "host": "fixture.invalid",
                "username": "fixture", "authType": "key", "keyId": key.id,
            }))
            .unwrap(),
        );
        store
            .trust("fixture.invalid", 22, &key.fingerprint)
            .unwrap();
        store.add_key(key).unwrap();
        snippets::save(
            &mut store.data.snippets,
            snippets::Snippet {
                name: "Inspect workspace".into(),
                script: "pwd\nls -la".into(),
                tags: vec!["workspace".into(), "inspect".into()],
                ..Default::default()
            },
            1_700_000_000_000,
        )
        .unwrap();
        store.persist().unwrap();
        let mut book = logs::LogBook::open(dir.path()).unwrap();
        let id = book
            .start("fixture-host", "Fixture host", "fixture.invalid:22", "ssh")
            .unwrap();
        book.update(&id, logs::FAILED, Some("Connection timed out"))
            .unwrap();
        book.finish(&id).unwrap();

        let mut context = tauri::generate_context!();
        context.config_mut().build.dev_url = Some(url.parse().unwrap());
        context.config_mut().app.windows[0].url = tauri::WebviewUrl::External(url.parse().unwrap());
        context.config_mut().app.windows[0].title = "OpenTermius page smoke".into();
        context.config_mut().app.windows[0].data_directory = Some(dir.path().join("webview"));
        tauri::Builder::default()
            .any_thread()
            .plugin(tauri_plugin_dialog::init())
            .manage(AppState {
                store: Mutex::new(store),
                logs: Mutex::new(Ok(book)),
                sessions: crate::sessions::SessionManager::default(),
            })
            .invoke_handler(tauri::generate_handler![
                crate::hosts::list_hosts,
                crate::keychain::list_keys,
                crate::keychain::generate_key,
                crate::keychain::key_requires_passphrase,
                crate::keychain::import_key,
                crate::keychain::rename_key,
                crate::keychain::delete_key,
                crate::keychain::export_public_key,
                crate::snippets::list_snippets,
                crate::snippets::save_snippet,
                crate::snippets::delete_snippet,
                crate::known_hosts::list_known_hosts,
                crate::known_hosts::delete_known_host,
                crate::known_hosts::import_known_hosts,
                list_logs,
                clear_logs,
            ])
            .run(context)
            .unwrap();
    }
}
