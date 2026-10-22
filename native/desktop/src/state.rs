//! State shared by all commands.
use crate::sessions::SessionManager;
use opentermius::store::Store;
use std::sync::{Mutex, MutexGuard};

pub struct AppState {
    /// The local configuration; lock briefly and never across `.await`.
    pub store: Mutex<Store>,
    pub sessions: SessionManager,
}

impl AppState {
    pub fn open() -> anyhow::Result<Self> {
        let store = opentermius::store::open_default()?;
        let sessions = SessionManager::new(store.path.parent().unwrap_or(std::path::Path::new(".")))?;
        Ok(Self { store: Mutex::new(store), sessions })
    }

    pub fn store(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())
    }
}
