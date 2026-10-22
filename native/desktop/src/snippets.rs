//! Snippet editing delegates validation to the core and rolls back failed writes.
use crate::state::AppState;
use opentermius::{
    snippets::{self, Snippet},
    store::Store,
};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetDraft {
    id: String,
    name: String,
    script: String,
    tags: String,
}

fn persist<T>(
    store: &mut Store,
    mutate: impl FnOnce(&mut Vec<Snippet>) -> anyhow::Result<T>,
) -> Result<T, String> {
    let previous = store.data.snippets.clone();
    let result = mutate(&mut store.data.snippets).and_then(|value| store.persist().map(|_| value));
    result.map_err(|error| {
        store.data.snippets = previous;
        format!("{error:#}")
    })
}

#[tauri::command]
pub fn list_snippets(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<Snippet>, String> {
    Ok(snippets::search(&state.store()?.data.snippets, &query)
        .into_iter()
        .cloned()
        .collect())
}

#[tauri::command]
pub fn save_snippet(
    state: tauri::State<'_, AppState>,
    draft: SnippetDraft,
) -> Result<String, String> {
    let snippet = Snippet {
        id: draft.id,
        name: draft.name,
        script: draft.script,
        tags: snippets::parse_tags(&draft.tags),
        ..Default::default()
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    persist(&mut *state.store()?, |list| {
        snippets::save(list, snippet, now)
    })
}

#[tauri::command]
pub fn delete_snippet(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    persist(&mut *state.store()?, |list| {
        anyhow::ensure!(
            snippets::delete(list, &id),
            "{}",
            opentermius::i18n::t("Snippet not found")
        );
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_mutations_and_failed_writes_restore_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("config.json")).unwrap();
        let draft = Snippet {
            name: "Fixture".into(),
            script: "pwd".into(),
            ..Default::default()
        };
        persist(&mut store, |list| snippets::save(list, draft.clone(), 1)).unwrap();
        let original = store.data.snippets.clone();
        let result: Result<(), _> = persist(&mut store, |list| {
            list.clear();
            anyhow::bail!("fixture failure")
        });
        assert!(result.is_err());
        assert_eq!(store.data.snippets, original);
        std::fs::remove_file(&store.path).unwrap();
        std::fs::create_dir(&store.path).unwrap();
        assert!(persist(&mut store, |list| snippets::save(list, draft, 2)).is_err());
        assert_eq!(store.data.snippets, original);
    }
}
