//! Session ownership, credential prompts, and the desktop event contract.
use crate::state::AppState;
use base64::{Engine, engine::general_purpose::STANDARD};
use opentermius::{
    hosts,
    i18n::t,
    local,
    logs::{self, LogBook},
    ssh,
    store::{self, Profile, Store},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    session_id: String,
    host_id: String,
    title: String,
    local: bool,
    connected: bool,
    ended: bool,
}

/// Typed secrets exist only until the attempt succeeds, fails, or is cancelled.
#[derive(Default)]
struct Credentials {
    secret: Option<Zeroizing<String>>,
}
impl Credentials {
    fn typed(&mut self, secret: String) {
        self.secret = (!secret.is_empty()).then(|| Zeroizing::new(secret));
    }
    fn remember(&mut self, store: &mut Store, profile: &Profile) -> anyhow::Result<()> {
        let secret = self.secret.take();
        let Some(mut saved) = store
            .data
            .hosts
            .iter()
            .find(|p| p.id == profile.id)
            .cloned()
        else {
            return Ok(());
        };
        let new_username = saved.username.is_empty() && !profile.username.is_empty();
        if secret.is_none() && !new_username {
            return Ok(());
        }
        if new_username {
            saved.username.clone_from(&profile.username);
        }
        store.save(saved, secret.as_deref().map(String::as_str), true)
    }
}

struct Entry {
    info: SessionInfo,
    profile: Option<Profile>,
    credentials: Credentials,
    transport: Option<ssh::Session>,
    generation: u64,
    size: (u16, u16),
    log_id: Option<String>,
    failed: bool,
}
enum Reply {
    Username,
    Credential,
    HostKey {
        host: String,
        port: u16,
        fingerprint: String,
        reply: oneshot::Sender<bool>,
    },
    Interactive {
        count: usize,
        reply: oneshot::Sender<Option<Vec<String>>>,
    },
}
struct PendingPrompt {
    session_id: String,
    reply: Reply,
}
struct Table {
    entries: HashMap<String, Entry>,
    order: Vec<String>,
    prompts: HashMap<String, PendingPrompt>,
    log_directory: PathBuf,
    initial_size: (u16, u16),
}
pub struct SessionManager {
    runtime: Option<Runtime>,
    table: Mutex<Table>,
}

fn session_error() -> String {
    t("Session not found").into()
}
fn translated(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl SessionManager {
    pub fn new(directory: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            runtime: Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(8)
                    .enable_all()
                    .build()?,
            ),
            table: Mutex::new(Table {
                entries: HashMap::new(),
                order: Vec::new(),
                prompts: HashMap::new(),
                log_directory: directory.to_owned(),
                initial_size: (100, 30),
            }),
        })
    }
    fn table(&self) -> Result<MutexGuard<'_, Table>, String> {
        self.table
            .lock()
            .map_err(|_| t("Session table is unavailable").into())
    }
    /// Used by SFTP and snippets as well as terminal input commands.
    pub fn send(&self, session_id: &str, command: ssh::Command) -> Result<(), String> {
        let table = self.table()?;
        let session = table
            .entries
            .get(session_id)
            .and_then(|e| e.transport.as_ref())
            .ok_or_else(session_error)?;
        session
            .commands
            .try_send(command)
            .map_err(|_| t("Session input queue is unavailable; try again").into())
    }
    fn create(
        &self,
        app: &AppHandle,
        state: &AppState,
        profile: Option<Profile>,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut table = self.table()?;
        let is_local = profile.is_none();
        if table.entries.len() >= 32 {
            return Err(t("Close an unused session before opening more than 32 tabs").into());
        }
        if is_local && table.entries.values().filter(|e| e.info.local).count() >= 8 {
            return Err(t("Close an unused terminal before opening another local session").into());
        }
        let info = SessionInfo {
            session_id: id.clone(),
            host_id: profile.as_ref().map(|p| p.id.clone()).unwrap_or_default(),
            title: profile
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| t("Local Terminal").into()),
            local: is_local,
            connected: false,
            ended: false,
        };
        let size = table.initial_size;
        table.entries.insert(
            id.clone(),
            Entry {
                info,
                profile,
                credentials: Credentials::default(),
                transport: None,
                generation: 0,
                size,
                log_id: None,
                failed: false,
            },
        );
        table.order.push(id.clone());
        if let Err(error) = self.credentials(app, state, &mut table, &id) {
            Self::fail(app, &mut table, &id, &error);
        }
        Ok(id)
    }
    fn prompt(
        app: &AppHandle,
        table: &mut Table,
        id: &str,
        kind: &str,
        title: &str,
        message: String,
        extra: Value,
        reply: Reply,
    ) {
        let prompt_id = uuid::Uuid::new_v4().to_string();
        let mut payload = json!({"promptId":prompt_id,"sessionId":id,"kind":kind,"title":title,"message":message});
        if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
            payload.extend(extra.clone());
        }
        table.prompts.insert(
            prompt_id,
            PendingPrompt {
                session_id: id.into(),
                reply,
            },
        );
        let _ = app.emit("session://prompt", payload);
    }
    fn credentials(
        &self,
        app: &AppHandle,
        state: &AppState,
        table: &mut Table,
        id: &str,
    ) -> Result<(), String> {
        let entry = table.entries.get(id).ok_or_else(session_error)?;
        let Some(profile) = &entry.profile else {
            return self.start(app, state, table, id, String::new());
        };
        if profile.username.is_empty() {
            Self::prompt(
                app,
                table,
                id,
                "username",
                t("Username"),
                t("Username").into(),
                json!({}),
                Reply::Username,
            );
            return Ok(());
        }
        if profile.auth_type == "key" && !profile.key_id.is_empty() {
            return self.start(app, state, table, id, String::new());
        }
        if let Some(encrypted) = &profile.encrypted_secret {
            match store::unprotect(encrypted) {
                Ok(secret) => return self.start(app, state, table, id, secret),
                Err(error) => Self::status(
                    app,
                    id,
                    false,
                    false,
                    true,
                    &opentermius::tf!("Cannot read saved credentials; enter them again: {e}", "e" => error.to_string()),
                ),
            }
        }
        let key = profile.auth_type == "key";
        let title = if key {
            t("Private key passphrase (leave empty for an unencrypted key)")
        } else {
            t("Password")
        };
        Self::prompt(
            app,
            table,
            id,
            if key { "passphrase" } else { "password" },
            title,
            title.into(),
            json!({}),
            Reply::Credential,
        );
        Ok(())
    }
    fn start(
        &self,
        app: &AppHandle,
        state: &AppState,
        table: &mut Table,
        id: &str,
        secret: String,
    ) -> Result<(), String> {
        let secret = Zeroizing::new(secret);
        let entry = table.entries.get_mut(id).ok_or_else(session_error)?;
        let runtime = self
            .runtime
            .as_ref()
            .expect("session runtime exists until drop");
        let (events, mut received) = mpsc::channel(128);
        let (host_id, title, address, kind) = if let Some(profile) = &entry.profile {
            (
                profile.id.as_str(),
                profile.name.as_str(),
                hosts::copy_address(profile),
                "ssh",
            )
        } else {
            ("", t("Local Terminal"), t("Local").into(), "local")
        };
        entry.log_id = match with_history(&table.log_directory, |book| {
            book.start(host_id, title, &address, kind)
        }) {
            Ok(log_id) => Some(log_id),
            Err(error) => {
                Self::history_error(app, id, false, &error);
                None
            }
        };
        let transport = if let Some(profile) = &entry.profile {
            let store = state.store()?;
            let settings = store.data.settings.clone().normalized();
            let key_material = if profile.auth_type == "key" && !profile.key_id.is_empty() {
                Some(store.key_material(&profile.key_id).map_err(translated)?)
            } else {
                None
            };
            let known = store
                .data
                .known_hosts
                .get(&store::key(&profile.host, profile.port))
                .cloned();
            ssh::spawn_with_key_material_and_options(
                runtime.handle(),
                id.into(),
                profile.clone(),
                secret.to_string(),
                key_material,
                known,
                events,
                ssh::ConnectOptions {
                    keepalive_seconds: settings.keepalive_seconds,
                    timeout_seconds: settings.connection_timeout_seconds,
                    initial_size: entry.size,
                },
            )
        } else {
            local::spawn_with_size(runtime.handle(), id.into(), events, entry.size)
        };
        entry.transport = Some(transport);
        let generation = entry.generation;
        let session_id = id.to_string();
        let app = app.clone();
        Self::status(&app, id, false, false, false, t("Connecting"));
        runtime.spawn(async move {
            while let Some(event) = received.recv().await {
                let state = app.state::<AppState>();
                state
                    .sessions
                    .event(&app, &state, &session_id, generation, event);
            }
        });
        Ok(())
    }
    fn history_error(app: &AppHandle, id: &str, connected: bool, error: &anyhow::Error) {
        Self::status(
            app,
            id,
            connected,
            false,
            true,
            &opentermius::tf!(
            "Cannot read or write connection history: {error}", "error" => error.to_string()),
        );
    }
    fn status(app: &AppHandle, id: &str, connected: bool, ended: bool, error: bool, message: &str) {
        let _ = app.emit("session://status", json!({"sessionId":id,"connected":connected,"ended":ended,"error":error,"message":message}));
    }
    fn fail(app: &AppHandle, table: &mut Table, id: &str, message: &str) {
        if let Some(entry) = table.entries.get_mut(id) {
            entry.credentials.secret = None;
            entry.failed = true;
            entry.info.connected = false;
            entry.info.ended = true;
            if let Some(log_id) = &entry.log_id {
                // Persist a known category, never arbitrary server text or echoed credentials.
                let _ = with_history(&table.log_directory, |book| {
                    book.update(
                        log_id,
                        logs::FAILED,
                        Some("Connection failed; see the error in the session"),
                    )
                });
            }
        }
        table.prompts.retain(|_, p| p.session_id != id);
        Self::status(app, id, false, true, true, message);
    }
    fn event(
        &self,
        app: &AppHandle,
        state: &AppState,
        id: &str,
        generation: u64,
        event: ssh::Event,
    ) {
        let Ok(mut table) = self.table() else {
            return;
        };
        // Reconnect keeps the tab id; old transport events belong to the old attempt.
        if !table
            .entries
            .get(id)
            .is_some_and(|e| e.generation == generation)
        {
            return;
        }
        match event {
            ssh::Event::Output { data, .. } => {
                let _ = app.emit(
                    "session://output",
                    json!({"sessionId":id,"data":STANDARD.encode(data)}),
                );
            }
            ssh::Event::Status {
                connected,
                message,
                error,
                ..
            } => {
                if error {
                    Self::fail(app, &mut table, id, &message);
                    return;
                }
                let entry = table.entries.get_mut(id).expect("entry checked above");
                let first_connected = connected && !entry.info.connected;
                entry.info.connected = connected;
                entry.info.ended = false;
                let mut warning = None;
                if first_connected {
                    if let Some(profile) = &entry.profile {
                        let result = state.store().and_then(|mut store| {
                            entry
                                .credentials
                                .remember(&mut store, profile)
                                .map_err(translated)?;
                            hosts::mark_connected(&mut store, &profile.id, &hosts::now_rfc3339())
                                .map_err(translated)?;
                            Ok(())
                        });
                        if let Err(error) = result {
                            warning = Some(
                                opentermius::tf!("Connected, but cannot save the credentials: {error}", "error" => error),
                            );
                        }
                    }
                    if let Some(log_id) = entry.log_id.clone() {
                        if let Err(error) = with_history(&table.log_directory, |book| {
                            book.update(&log_id, logs::CONNECTED, None)
                        }) {
                            Self::history_error(app, id, true, &error);
                        }
                    }
                }
                Self::status(
                    app,
                    id,
                    connected,
                    false,
                    warning.is_some(),
                    warning.as_deref().unwrap_or(&message),
                );
            }
            ssh::Event::HostKey {
                host,
                port,
                fingerprint,
                reply,
                ..
            } => {
                let message = opentermius::tf!("{host}:{port}\n\n{fingerprint}\n\nVerify this fingerprint through a trusted channel before trusting it.",
                    "host" => host.clone(), "port" => port.to_string(), "fingerprint" => fingerprint.clone());
                Self::prompt(
                    app,
                    &mut table,
                    id,
                    "hostKey",
                    t("First connection · Verify host key"),
                    message,
                    json!({"fingerprint":fingerprint}),
                    Reply::HostKey {
                        host,
                        port,
                        fingerprint,
                        reply,
                    },
                );
            }
            ssh::Event::AuthPrompt { prompts, reply, .. } => {
                let count = prompts.len();
                let prompts: Vec<_> = prompts
                    .into_iter()
                    .map(|(prompt, echo)| json!({"prompt":prompt,"echo":echo}))
                    .collect();
                Self::prompt(
                    app,
                    &mut table,
                    id,
                    "keyboardInteractive",
                    t("Waiting for authentication"),
                    t("Waiting for authentication").into(),
                    json!({"prompts":prompts}),
                    Reply::Interactive { count, reply },
                );
            }
            ssh::Event::Stats { stats, .. } => {
                let host_id = &table.entries[id].info.host_id;
                let _ = app.emit("session://stats", json!({"sessionId":id,"hostId":host_id,"cpu":stats.cpu,"memory":stats.memory,"disk":stats.disk}));
            }
            ssh::Event::Closed { .. } => {
                let entry = table.entries.get_mut(id).expect("entry checked above");
                entry.credentials.secret = None;
                entry.transport.take();
                entry.info.connected = false;
                entry.info.ended = true;
                let failed = entry.failed;
                if let Some(log_id) = entry.log_id.take() {
                    let _ = with_history(&table.log_directory, |book| book.finish(&log_id));
                }
                table.prompts.retain(|_, p| p.session_id != id);
                if !failed {
                    Self::status(app, id, false, true, false, t("Disconnected"));
                }
                let _ = app.emit("session://closed", json!({"sessionId":id}));
            }
            event => {
                if let Some((name, payload)) = sftp_payload(event) {
                    let _ = app.emit(name, payload);
                }
            }
        }
    }
    fn answer(
        &self,
        app: &AppHandle,
        state: &AppState,
        prompt_id: &str,
        accepted: bool,
        answers: Vec<String>,
    ) -> Result<(), String> {
        let mut answers = Zeroizing::new(answers);
        let mut table = self.table()?;
        let pending = table
            .prompts
            .get(prompt_id)
            .ok_or_else(|| t("Authentication prompt has expired").to_string())?;
        validate_answers(&pending.reply, accepted, &answers)?;
        let pending = table
            .prompts
            .remove(prompt_id)
            .expect("prompt checked above");
        let id = pending.session_id;
        match pending.reply {
            Reply::Username => {
                if !accepted {
                    Self::fail(app, &mut table, &id, t("Cancelled"));
                    return Ok(());
                }
                let profile = table
                    .entries
                    .get_mut(&id)
                    .and_then(|e| e.profile.as_mut())
                    .ok_or_else(session_error)?;
                profile.username = answers[0].trim().into();
                if let Err(error) = self.credentials(app, state, &mut table, &id) {
                    Self::fail(app, &mut table, &id, &error);
                }
            }
            Reply::Credential => {
                if !accepted {
                    Self::fail(app, &mut table, &id, t("Cancelled"));
                    return Ok(());
                }
                let secret = std::mem::take(&mut answers[0]);
                table
                    .entries
                    .get_mut(&id)
                    .ok_or_else(session_error)?
                    .credentials
                    .typed(secret.clone());
                if let Err(error) = self.start(app, state, &mut table, &id, secret) {
                    Self::fail(app, &mut table, &id, &error);
                }
            }
            Reply::HostKey {
                host,
                port,
                fingerprint,
                reply,
            } => {
                let result = if accepted {
                    state
                        .store()?
                        .trust(&host, port, &fingerprint)
                        .map_err(translated)
                } else {
                    Ok(())
                };
                let _ = reply.send(accepted && result.is_ok());
                result?;
            }
            Reply::Interactive { reply, .. } => {
                let _ = reply.send(accepted.then(|| std::mem::take(&mut *answers)));
            }
        }
        Ok(())
    }
    fn close(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let mut table = self.table()?;
        let mut entry = table.entries.remove(id).ok_or_else(session_error)?;
        table.order.retain(|candidate| candidate != id);
        table.prompts.retain(|_, p| p.session_id != id);
        if let Some(session) = entry.transport.take() {
            session.task.abort();
        }
        if let Some(log_id) = entry.log_id.take() {
            if let Err(error) = with_history(&table.log_directory, |book| book.finish(&log_id)) {
                Self::history_error(app, id, false, &error);
            }
        }
        Self::status(app, id, false, true, false, t("Disconnected"));
        let _ = app.emit("session://closed", json!({"sessionId":id}));
        Ok(())
    }
    fn reconnect(&self, app: &AppHandle, state: &AppState, id: &str) -> Result<(), String> {
        let mut table = self.table()?;
        table.prompts.retain(|_, p| p.session_id != id);
        let entry = table.entries.get_mut(id).ok_or_else(session_error)?;
        if let Some(session) = entry.transport.take() {
            session.task.abort();
        }
        let log_id = entry.log_id.take();
        entry.generation += 1;
        entry.credentials.secret = None;
        entry.info.connected = false;
        entry.info.ended = false;
        entry.failed = false;
        if let Some(profile) = &mut entry.profile {
            if let Some(saved) = state
                .store()?
                .data
                .hosts
                .iter()
                .find(|p| p.id == profile.id)
            {
                *profile = saved.clone();
            } else {
                profile.encrypted_secret = None;
            }
        }
        if let Some(log_id) = log_id {
            if let Err(error) = with_history(&table.log_directory, |book| book.finish(&log_id)) {
                Self::history_error(app, id, false, &error);
            }
        }
        Self::status(
            app,
            id,
            false,
            false,
            false,
            t("Waiting for authentication"),
        );
        if let Err(error) = self.credentials(app, state, &mut table, id) {
            Self::fail(app, &mut table, id, &error);
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let Ok(mut table) = self.table() else {
            return;
        };
        table.prompts.clear();
        table.order.clear();
        for (_, mut entry) in std::mem::take(&mut table.entries) {
            if let Some(session) = entry.transport.take() {
                session.task.abort();
            }
            if let Some(log_id) = entry.log_id.take() {
                let _ = with_history(&table.log_directory, |book| book.finish(&log_id));
            }
        }
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.shutdown();
        // Tauri may release state on an async thread; blocking runtime teardown would panic.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

// Reopen for each mutation so clearing history in another desktop module is
// respected; a cached LogBook would resurrect the cleared entries on disconnect.
fn with_history<T>(
    directory: &Path,
    action: impl FnOnce(&mut LogBook) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    action(&mut LogBook::open(directory)?)
}

fn validate_answers(reply: &Reply, accepted: bool, answers: &[String]) -> Result<(), String> {
    let expired = match reply {
        Reply::HostKey { reply, .. } => reply.is_closed(),
        Reply::Interactive { reply, .. } => reply.is_closed(),
        _ => false,
    };
    if expired {
        return Err(t("Authentication prompt has expired").into());
    }
    if !accepted {
        return Ok(());
    }
    let count = match reply {
        Reply::HostKey { .. } => return Ok(()),
        Reply::Interactive { count, .. } => *count,
        _ => 1,
    };
    if answers.len() != count || answers.iter().any(|a| a.len() > 65536) {
        return Err(t("Invalid authentication answers").into());
    }
    if matches!(reply, Reply::Username) {
        let username = answers[0].trim();
        if username.is_empty()
            || username.chars().count() > 128
            || username.len() > 512
            || answers[0].chars().any(char::is_control)
        {
            return Err(t("Invalid Username").into());
        }
    }
    Ok(())
}

/// `sessionId` routes desktop events; `id` retains the core variant's identifier.
fn sftp_payload(event: ssh::Event) -> Option<(&'static str, Value)> {
    Some(match event {
        ssh::Event::Files { id, path, entries } => (
            "sftp://files",
            json!({"sessionId":id,"id":id,"path":path,"entries":entries}),
        ),
        ssh::Event::Transfer { id, message, error } => (
            "sftp://transfer",
            json!({"sessionId":id,"id":id,"message":message,"error":error}),
        ),
        ssh::Event::TransferProgress {
            id,
            transfer,
            name,
            direction,
            transferred,
            total,
            speed,
        } => (
            "sftp://progress",
            json!({"sessionId":id,"id":id,"transfer":transfer,"name":name,"direction":direction,"transferred":transferred,"total":total,"speed":speed}),
        ),
        ssh::Event::TransferFinished {
            id,
            transfer,
            message,
            error,
            cancelled,
        } => (
            "sftp://finished",
            json!({"sessionId":id,"id":id,"transfer":transfer,"message":message,"error":error,"cancelled":cancelled}),
        ),
        ssh::Event::OverwriteRequired {
            id,
            local_root,
            directory,
            local,
            transfer,
            folder,
        } => (
            "sftp://overwrite",
            json!({"sessionId":id,"id":id,"localRoot":local_root,"directory":directory,"local":local,"transfer":transfer,"folder":folder}),
        ),
        ssh::Event::DeleteRequired {
            id,
            directory,
            name,
            files,
            dirs,
            bytes,
        } => (
            "sftp://delete",
            json!({"sessionId":id,"id":id,"directory":directory,"name":name,"files":files,"dirs":dirs,"bytes":bytes}),
        ),
        _ => return None,
    })
}

#[tauri::command]
pub fn connect_host(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
) -> Result<String, String> {
    let profile = state
        .store()?
        .data
        .hosts
        .iter()
        .find(|p| p.id == host_id)
        .cloned()
        .ok_or_else(|| t("Host not found").to_string())?;
    state.sessions.create(&app, &state, Some(profile))
}
#[tauri::command]
pub fn open_local_terminal(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    state.sessions.create(&app, &state, None)
}
#[tauri::command]
pub fn reconnect_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state.sessions.reconnect(&app, &state, &session_id)
}
#[tauri::command]
pub fn duplicate_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let mut profile = state
        .sessions
        .table()?
        .entries
        .get(&session_id)
        .ok_or_else(session_error)?
        .profile
        .clone();
    if let Some(previous) = &mut profile {
        if let Some(saved) = state
            .store()?
            .data
            .hosts
            .iter()
            .find(|p| p.id == previous.id)
        {
            *previous = saved.clone();
        }
    }
    state.sessions.create(&app, &state, profile)
}
#[tauri::command]
pub fn session_input(
    state: State<'_, AppState>,
    session_id: String,
    mut data: String,
) -> Result<(), String> {
    let result = state
        .sessions
        .send(&session_id, ssh::Command::Input(data.as_bytes().to_vec()));
    data.zeroize();
    result
}
#[tauri::command]
pub fn session_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut table = state.sessions.table()?;
    let entry = table
        .entries
        .get_mut(&session_id)
        .ok_or_else(session_error)?;
    // Hidden panels report zero dimensions; retain the last visible size.
    if cols < 2 || rows < 2 {
        return Ok(());
    }
    entry.size = (cols.min(500), rows.min(300));
    if let Some(session) = &entry.transport {
        session
            .commands
            .try_send(ssh::Command::Resize {
                cols: entry.size.0,
                rows: entry.size.1,
            })
            .map_err(|_| t("Session input queue is unavailable; try again").to_string())?;
    }
    table.initial_size = (cols.min(500), rows.min(300));
    Ok(())
}
#[tauri::command]
pub fn close_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state.sessions.close(&app, &session_id)
}
#[tauri::command]
pub fn answer_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt_id: String,
    accepted: bool,
    answers: Vec<String>,
) -> Result<(), String> {
    state
        .sessions
        .answer(&app, &state, &prompt_id, accepted, answers)
}
#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let table = state.sessions.table()?;
    Ok(table
        .order
        .iter()
        .filter_map(|id| table.entries.get(id).map(|e| e.info.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_validation_preserves_multifield_and_empty_password_answers() {
        assert!(validate_answers(&Reply::Username, true, &["  test-fixture  ".into()]).is_ok());
        for value in ["", " \t", "bad\0value", "bad\nvalue"] {
            assert!(validate_answers(&Reply::Username, true, &[value.into()]).is_err());
        }
        assert!(validate_answers(&Reply::Username, true, &["x".repeat(129)]).is_err());
        assert!(validate_answers(&Reply::Credential, true, &[String::new()]).is_ok());
        assert!(validate_answers(&Reply::Credential, true, &[]).is_err());
        assert!(validate_answers(&Reply::Credential, false, &[]).is_ok());
        let (reply, _received) = oneshot::channel();
        let prompt = Reply::Interactive { count: 2, reply };
        assert!(validate_answers(&prompt, true, &["one".into(), "two".into()]).is_ok());
        assert!(validate_answers(&prompt, true, &["one".into()]).is_err());
    }
    #[test]
    fn typed_credentials_are_consumed_only_by_successful_login() {
        let directory = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let mut store = Store::open(directory.join("config.json")).unwrap();
        let mut saved = store::parse_quick_connect("fixture.invalid").unwrap();
        saved.username.clear();
        store.save(saved.clone(), None, true).unwrap();
        let mut attempt = saved;
        attempt.username = "test-fixture".into();
        let mut credentials = Credentials::default();
        credentials.typed("fixture-password".into());
        assert!(store.data.hosts[0].encrypted_secret.is_none());
        assert!(store.data.hosts[0].username.is_empty());
        credentials.remember(&mut store, &attempt).unwrap();
        assert!(credentials.secret.is_none());
        assert_eq!(store.data.hosts[0].username, attempt.username);
        assert_eq!(
            store::unprotect(store.data.hosts[0].encrypted_secret.as_ref().unwrap()).unwrap(),
            "fixture-password"
        );
        credentials.typed("wrong-fixture-password".into());
        credentials.secret = None;
        assert_eq!(
            store::unprotect(store.data.hosts[0].encrypted_secret.as_ref().unwrap()).unwrap(),
            "fixture-password"
        );
        let path = store.path.clone();
        store.path = path.join("cannot-write.json");
        credentials.typed("unsaved-fixture-password".into());
        assert!(credentials.remember(&mut store, &attempt).is_err());
        assert!(credentials.secret.is_none());
        store.path = path;
        assert_eq!(
            store::unprotect(store.data.hosts[0].encrypted_secret.as_ref().unwrap()).unwrap(),
            "fixture-password"
        );
        attempt.username = "another-fixture".into();
        credentials.remember(&mut store, &attempt).unwrap();
        assert_eq!(store.data.hosts[0].username, "test-fixture");
        store.delete(&attempt.id).unwrap();
        credentials.typed("deleted-host-fixture-password".into());
        credentials.remember(&mut store, &attempt).unwrap();
        assert!(credentials.secret.is_none());
        assert!(store.data.hosts.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn dropping_session_prompts_releases_transport_waiters() {
        let (reply, mut received) = oneshot::channel();
        let mut prompts = HashMap::new();
        prompts.insert(
            "prompt",
            PendingPrompt {
                session_id: "session".into(),
                reply: Reply::Interactive { count: 1, reply },
            },
        );
        prompts.retain(|_, p| p.session_id != "session");
        assert!(matches!(
            received.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }
    #[test]
    fn expired_transport_prompt_cannot_be_accepted() {
        let (reply, received) = oneshot::channel();
        drop(received);
        let prompt = Reply::HostKey {
            host: "fixture.invalid".into(),
            port: 22,
            fingerprint: "fixture".into(),
            reply,
        };
        assert!(validate_answers(&prompt, true, &[]).is_err());
    }
    #[test]
    fn history_mutations_respect_clear_from_other_commands() {
        let directory = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let old = with_history(&directory, |book| {
            book.start("", "fixture", "local", "local")
        })
        .unwrap();
        LogBook::open(&directory).unwrap().clear().unwrap();
        let current = with_history(&directory, |book| {
            book.start("", "fixture", "local", "local")
        })
        .unwrap();
        with_history(&directory, |book| book.finish(&old)).unwrap();
        let book = LogBook::open(&directory).unwrap();
        assert_eq!(book.entries().len(), 1);
        assert_eq!(book.entries()[0].id, current);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn shutdown_cancels_owned_tasks_and_prompts_and_finishes_history() {
        let directory = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = SessionManager::new(&directory).unwrap();
        let runtime = manager.runtime.as_ref().unwrap();
        let task = runtime.spawn(std::future::pending());
        let aborted = task.abort_handle();
        let (commands, _received) = mpsc::channel(1);
        let (reply, mut answer) = oneshot::channel();
        let log_id = with_history(&directory, |book| {
            book.start("", "fixture", "local", "local")
        })
        .unwrap();
        {
            let mut table = manager.table().unwrap();
            table.order.push("session".into());
            table.prompts.insert(
                "prompt".into(),
                PendingPrompt {
                    session_id: "session".into(),
                    reply: Reply::Interactive { count: 1, reply },
                },
            );
            table.entries.insert(
                "session".into(),
                Entry {
                    info: SessionInfo {
                        session_id: "session".into(),
                        host_id: String::new(),
                        title: "fixture".into(),
                        local: true,
                        connected: true,
                        ended: false,
                    },
                    profile: None,
                    credentials: Credentials::default(),
                    transport: Some(ssh::Session {
                        id: "session".into(),
                        commands,
                        task,
                    }),
                    generation: 0,
                    size: (100, 30),
                    log_id: Some(log_id),
                    failed: false,
                },
            );
        }
        manager.shutdown();
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while !aborted.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
        assert!(manager.table().unwrap().entries.is_empty());
        assert!(manager.table().unwrap().order.is_empty());
        assert!(matches!(
            answer.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert_eq!(
            LogBook::open(&directory).unwrap().entries()[0].status,
            logs::CLOSED
        );
        assert!(
            manager
                .send("session", ssh::Command::Input(vec![0]))
                .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn sftp_fields_are_camel_case_and_keep_transfer_metadata() {
        let (name, payload) = sftp_payload(ssh::Event::OverwriteRequired {
            id: "session".into(),
            local_root: "fixture-root".into(),
            directory: "/fixture".into(),
            local: "fixture-root/file".into(),
            transfer: 42,
            folder: true,
        })
        .unwrap();
        assert_eq!(name, "sftp://overwrite");
        assert_eq!(payload["sessionId"], "session");
        assert_eq!(payload["transfer"], 42);
        assert_eq!(payload["localRoot"], "fixture-root");
        assert_eq!(payload["folder"], true);
        assert!(payload.get("local_root").is_none());
        let (_, payload) = sftp_payload(ssh::Event::Files {
            id: "session".into(),
            path: "/".into(),
            entries: vec![ssh::RemoteFile {
                name: "fixture".into(),
                path: "/fixture".into(),
                size: 0,
                is_dir: true,
                modified: String::new(),
                permissions: Some(0o755),
            }],
        })
        .unwrap();
        assert_eq!(payload["entries"][0]["isDir"], true);
    }
}
