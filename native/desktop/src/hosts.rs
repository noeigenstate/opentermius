//! Saved hosts for the Hosts page.
use crate::state::AppState;
use serde::Serialize;

/// One host as the Hosts page shows it; credentials never leave the backend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCard {
    id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    group: String,
}

#[tauri::command]
pub fn list_hosts(state: tauri::State<'_, AppState>) -> Result<Vec<HostCard>, String> {
    Ok(state
        .store()?
        .data
        .hosts
        .iter()
        .map(|p| HostCard {
            id: p.id.clone(),
            name: p.name.clone(),
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
            group: p.group.clone(),
        })
        .collect())
}
