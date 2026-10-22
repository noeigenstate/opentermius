//! State shared by all commands.
use crate::sessions::SessionManager;
use opentermius::store::Store;
use std::sync::{Mutex, MutexGuard};

pub struct AppState {
    /// Shared with session lifecycle hooks; history errors must not prevent startup.
    pub logs: Mutex<Result<opentermius::logs::LogBook, String>>,
    /// The local configuration; lock briefly and never across `.await`.
    pub store: Mutex<Store>,
    #[allow(dead_code)] // used once the sessions module lands
    pub sessions: SessionManager,
}

impl AppState {
    pub fn open() -> anyhow::Result<Self> {
        let store = opentermius::store::open_default()?;
        let logs = opentermius::logs::LogBook::open(store.path.parent().unwrap_or_else(|| std::path::Path::new(".")))
            .map_err(|error| opentermius::tf!("Cannot read or write connection history: {error}", "error" => format!("{error}")));
        Ok(Self { logs: Mutex::new(logs), store: Mutex::new(store), sessions: SessionManager::default() })
    }

    pub fn store(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())
    }
}
