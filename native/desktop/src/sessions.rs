//! Terminal sessions (SSH and local). The sessions worker fills this in; other modules
//! talk to a running session only through [`SessionManager::send`].
// Skeleton until the sessions module is implemented; remove this allowance then.
#![allow(dead_code)]
use opentermius::ssh;
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub struct SessionManager {
    /// Command channels of open SSH sessions, by session id.
    commands: Mutex<HashMap<String, tokio::sync::mpsc::Sender<ssh::Command>>>,
}

impl SessionManager {
    /// Queues a command (input, resize, SFTP job, …) for a session.
    pub fn send(&self, session_id: &str, command: ssh::Command) -> Result<(), String> {
        let commands = self.commands.lock().map_err(|_| "Session table is unavailable".to_string())?;
        let sender = commands.get(session_id).ok_or_else(|| opentermius::i18n::t("Session not found").to_string())?;
        sender.try_send(command).map_err(|error| error.to_string())
    }
}
