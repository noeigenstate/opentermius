//! Wiring for the Logs section: connection history storage and table model.
use crate::{AppWindow, LogRow, LogsBridge};
use chrono::{DateTime, Local};
use opentermius::logs::{self, LogBook, LogEntry};
use slint::{ComponentHandle, Model, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

struct LogsState {
    book: Option<LogBook>,
    /// Active session id -> history entry id.
    sessions: HashMap<String, String>,
    /// Last known host ids, used to show "Connect again" only for live hosts.
    hosts: HashSet<String>,
    rows: Rc<VecModel<LogRow>>,
    ui: slint::Weak<AppWindow>,
    error: String,
}

impl Default for LogsState {
    fn default() -> Self {
        Self {
            book: None,
            sessions: HashMap::new(),
            hosts: HashSet::new(),
            rows: Rc::new(VecModel::from(Vec::new())),
            ui: slint::Weak::default(),
            error: String::new(),
        }
    }
}

thread_local! {
    static LOGS: RefCell<LogsState> = RefCell::new(LogsState::default());
}

pub fn install(ui: &AppWindow) {
    let dir = crate::with_state(|state| {
        state
            .store
            .lock()
            .ok()
            .and_then(|store| store.path.parent().map(PathBuf::from))
    })
    .flatten()
    .unwrap_or_else(|| PathBuf::from("."));
    LOGS.with(|slot| {
        let mut state = slot.borrow_mut();
        state.book = storage_result(&mut state, LogBook::open(&dir));
        state.sessions.clear();
        state.ui = ui.as_weak();
    });
    let bridge = ui.global::<LogsBridge>();
    bridge.on_refresh(refresh);
    bridge.on_clear(|| {
        LOGS.with(|slot| {
            let mut state = slot.borrow_mut();
            if let Some(book) = state.book.as_mut() {
                let result = book.clear();
                let _ = storage_result(&mut state, result);
            }
        });
        refresh();
    });
    bridge.on_connect_again(|host_id| {
        crate::with_state(|state| state.connect(&host_id));
    });
    refresh();
}

fn storage_result<T>(state: &mut LogsState, result: anyhow::Result<T>) -> Option<T> {
    match result {
        Ok(value) => {
            state.error.clear();
            Some(value)
        }
        Err(error) => {
            state.error = format!("无法读写连接历史：{error}");
            None
        }
    }
}

/// Records the start of an SSH connection. Safe to call more than once for the
/// same session: only the first call creates a history row.
pub(crate) fn ssh_started(session_id: &str, profile: &opentermius::store::Profile) {
    let address = if profile.username.is_empty() {
        format!("{}:{}", profile.host, profile.port)
    } else {
        format!("{}@{}:{}", profile.username, profile.host, profile.port)
    };
    let label = if profile.name.is_empty() {
        &profile.host
    } else {
        &profile.name
    };
    begin(session_id, &profile.id, label, &address, "ssh");
}

/// Records the opening of a local terminal.
pub(crate) fn local_started(session_id: &str) {
    begin(session_id, "", "Local Terminal", "Local", "local");
}

fn begin(session_id: &str, host_id: &str, label: &str, address: &str, kind: &str) {
    let log_id = LOGS.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.sessions.contains_key(session_id) {
            return None;
        }
        let result = state.book.as_mut()?.start(host_id, label, address, kind);
        storage_result(&mut state, result)
    });
    if let Some(log_id) = log_id {
        LOGS.with(|slot| {
            slot.borrow_mut()
                .sessions
                .insert(session_id.to_string(), log_id);
        });
    }
    refresh();
}

pub(crate) fn connected(session_id: &str) {
    update(session_id, logs::CONNECTED, None);
}

pub(crate) fn failed(session_id: &str, message: &str) {
    // Transport errors may contain arbitrary server text. Persist only a known
    // category, never raw messages that could echo credentials or output.
    update(session_id, logs::FAILED, Some(failure_summary(message)));
}

fn failure_summary(message: &str) -> &'static str {
    if message.contains("主机密钥") {
        "SSH 主机密钥验证失败"
    } else if message.contains("私钥") || message.contains("口令") || message.contains("认证")
    {
        "SSH 身份验证失败"
    } else if message.contains("超时") || message.to_lowercase().contains("timed out") {
        "连接超时"
    } else if message.contains("取消") {
        "连接已取消"
    } else {
        "连接失败，请查看会话中的错误提示"
    }
}

fn update(session_id: &str, status: &str, error: Option<&str>) {
    LOGS.with(|slot| {
        let mut state = slot.borrow_mut();
        let Some(log_id) = state.sessions.get(session_id).cloned() else {
            return;
        };
        if let Some(book) = state.book.as_mut() {
            let result = book.update(&log_id, status, error);
            let _ = storage_result(&mut state, result);
        }
    });
    refresh();
}

pub(crate) fn closed(session_id: &str) {
    LOGS.with(|slot| {
        let mut state = slot.borrow_mut();
        let Some(log_id) = state.sessions.remove(session_id) else {
            return;
        };
        if let Some(book) = state.book.as_mut() {
            let result = book.finish(&log_id);
            let _ = storage_result(&mut state, result);
        }
    });
    refresh();
}

fn refresh() {
    // The page can be refreshed from inside `with_state` while a session event
    // is being handled; keep the previous host snapshot in that case.
    let hosts = crate::with_state(|state| {
        state.store.lock().ok().map(|store| {
            store
                .data
                .hosts
                .iter()
                .map(|profile| profile.id.clone())
                .collect::<HashSet<_>>()
        })
    })
    .flatten();
    let now = Local::now();
    let (rows, total) = LOGS.with(|slot| {
        let mut state = slot.borrow_mut();
        if let Some(hosts) = hosts {
            state.hosts = hosts;
        }
        let query = state
            .ui
            .upgrade()
            .map(|ui| ui.global::<LogsBridge>().get_query().trim().to_lowercase())
            .unwrap_or_default();
        let Some(book) = state.book.as_ref() else {
            return (Vec::new(), 0);
        };
        let entries = book.recent(logs::MAX_ENTRIES);
        let total = entries.len();
        let rows = entries
            .into_iter()
            .filter(|entry| matches_query(entry, &query))
            .map(|entry| row(&entry, &state.hosts, now))
            .collect();
        (rows, total)
    });
    let (model, ui, error) = LOGS.with(|slot| {
        let state = slot.borrow();
        (state.rows.clone(), state.ui.clone(), state.error.clone())
    });
    let Some(ui) = ui.upgrade() else {
        return;
    };
    if model.row_count() != rows.len() {
        model.set_vec(rows);
    } else {
        for (index, row) in rows.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
    }
    let bridge = ui.global::<LogsBridge>();
    bridge.set_rows(model.into());
    bridge.set_total(total as i32);
    bridge.set_storage_error(error.into());
}

fn matches_query(entry: &LogEntry, query: &str) -> bool {
    query.is_empty()
        || entry.label.to_lowercase().contains(query)
        || entry.address.to_lowercase().contains(query)
        || entry.status.contains(query)
        || entry
            .error
            .as_deref()
            .is_some_and(|error| error.to_lowercase().contains(query))
}

fn row(entry: &LogEntry, hosts: &HashSet<String>, now: DateTime<Local>) -> LogRow {
    LogRow {
        id: entry.id.clone().into(),
        host_id: entry.host_id.clone().into(),
        label: entry.label.clone().into(),
        address: entry.address.clone().into(),
        started: started_text(&entry.started_at).into(),
        duration: logs::elapsed_seconds(&entry.started_at, entry.ended_at.as_deref(), now)
            .map(logs::format_duration)
            .unwrap_or_else(|| "—".into())
            .into(),
        status: status_text(&entry.status).into(),
        error: entry.error.clone().unwrap_or_default().into(),
        can_connect: entry.kind == "ssh"
            && !entry.host_id.is_empty()
            && hosts.contains(&entry.host_id),
    }
}

fn started_text(value: &str) -> String {
    logs::parse_time(value)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| value.to_string())
}

fn status_text(status: &str) -> &'static str {
    match status {
        logs::CONNECTED => "Connected",
        logs::CLOSED => "Closed",
        logs::FAILED => "Failed",
        _ => "Connecting",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str) -> LogEntry {
        LogEntry {
            id: "log-1".into(),
            host_id: "host-1".into(),
            label: "Development".into(),
            address: "tester@example.invalid:22".into(),
            kind: kind.into(),
            started_at: "2026-01-01T10:00:00Z".into(),
            ended_at: Some("2026-01-01T10:01:05Z".into()),
            status: logs::FAILED.into(),
            error: Some("连接超时".into()),
        }
    }

    #[test]
    fn table_filters_metadata_and_only_reconnects_existing_ssh_hosts() {
        let ssh = entry("ssh");
        for query in ["", "develop", "example.invalid", "failed", "超时"] {
            assert!(matches_query(&ssh, query));
        }
        assert!(!matches_query(&ssh, "missing"));
        let hosts = HashSet::from(["host-1".to_string()]);
        let now = logs::parse_time("2026-01-01T10:02:00Z").unwrap();
        let ssh_row = row(&ssh, &hosts, now);
        assert!(ssh_row.can_connect);
        assert_eq!(ssh_row.duration, "1m 5s");
        assert_eq!(ssh_row.status, "Failed");
        assert!(!row(&ssh, &HashSet::new(), now).can_connect);
        assert!(!row(&entry("local"), &hosts, now).can_connect);
    }

    #[test]
    fn failure_history_never_copies_raw_transport_messages() {
        for message in [
            "server disconnected: password=private-secret",
            "私钥或口令无效: private-secret",
            "主机密钥验证失败: private-secret",
            "connection timed out: private-secret",
            "SSH 连接已取消: private-secret",
        ] {
            assert!(!failure_summary(message).contains("private-secret"));
        }
    }

    #[test]
    fn session_hooks_persist_lifecycle_and_keep_failed_status_after_close() {
        let temp = tempfile::tempdir().unwrap();
        LOGS.with(|slot| {
            *slot.borrow_mut() = LogsState {
                book: Some(LogBook::open(temp.path()).unwrap()),
                ..LogsState::default()
            };
        });
        local_started("session-1");
        local_started("session-1");
        connected("session-1");
        let entries = LogBook::open(temp.path()).unwrap().recent(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, logs::CONNECTED);
        assert_eq!(entries[0].address, "Local");
        closed("session-1");
        let entries = LogBook::open(temp.path()).unwrap().recent(10);
        assert_eq!(entries[0].status, logs::CLOSED);
        assert!(entries[0].ended_at.is_some());
        local_started("session-2");
        failed("session-2", "error containing private-secret");
        closed("session-2");
        // A late event from a removed session cannot resurrect its record.
        connected("session-2");
        let entries = LogBook::open(temp.path()).unwrap().recent(10);
        assert_eq!(entries[0].status, logs::FAILED);
        assert!(entries[0].ended_at.is_some());
        assert!(
            !std::fs::read_to_string(temp.path().join(logs::FILE_NAME))
                .unwrap()
                .contains("private-secret")
        );
        LOGS.with(|slot| assert!(slot.borrow().sessions.is_empty()));
    }
}
