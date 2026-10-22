//! State shared by all commands.
use crate::sessions::SessionManager;
use opentermius::store::Store;
use std::sync::{Mutex, MutexGuard};

pub struct AppState {
    /// The local configuration; lock briefly and never across `.await`.
    pub store: Mutex<Store>,
    #[allow(dead_code)] // used once the sessions module lands
    pub sessions: SessionManager,
}

impl AppState {
    pub fn open() -> anyhow::Result<Self> {
        Ok(Self { store: Mutex::new(opentermius::store::open_default()?), sessions: SessionManager::default() })
    }

    pub fn store(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())
    }
}
