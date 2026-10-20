#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Context;
use opentermius::{
    settings::{BellMode, Settings},
    ssh::{self, Command, Event},
    store::{self, Profile, Store},
    terminal::{self, Snapshot, Terminal},
};
use slint::Model;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroize;
slint::include_modules!();
mod pages;

struct SessionView {
    local: bool,
    profile: Profile,
    transport: Option<ssh::Session>,
    snapshot: Snapshot,
    status: String,
    connected: bool,
    ended: bool,
    selection: Option<((i32, i32), (i32, i32))>,
}
enum Prompt {
    Username {
        id: String,
    },
    Credential {
        id: String,
    },
    Host {
        id: String,
        host: String,
        port: u16,
        fingerprint: String,
        reply: oneshot::Sender<bool>,
    },
    Auth {
        id: String,
        prompts: Vec<(String, bool)>,
        index: usize,
        answers: Vec<String>,
        reply: oneshot::Sender<Option<Vec<String>>>,
    },
}
impl Prompt {
    fn session_id(&self) -> &str {
        match self {
            Self::Username { id } | Self::Credential { id }
            | Self::Host { id, .. } | Self::Auth { id, .. } => id,
        }
    }
}
struct State {
    ui: slint::Weak<AppWindow>,
    store: Arc<Mutex<Store>>,
    settings: Settings,
    runtime: tokio::runtime::Runtime,
    events: mpsc::Sender<Event>,
    sessions: BTreeMap<String, SessionView>,
    order: Vec<String>,
    active: Option<String>,
    queue: VecDeque<Prompt>,
    prompt: Option<Prompt>,
    cols: u16,
    rows: u16,
    file_path: String,
    search: String,
    group_filter: String,
    local_path: PathBuf,
    local_request: u64,
    pending_sftp: Option<String>,
    remote_entries: Vec<ssh::RemoteFile>,
    local_entries: Vec<ssh::LocalFile>,
    transfers: BTreeMap<u64, (String, TransferRow)>,
    next_transfer: u64,
    render_model: Rc<VecModel<TerminalRun>>,
    selecting: bool,
    mouse_button: Option<terminal::MouseButton>,
    forwards: BTreeMap<String, opentermius::forwarding::RunningRule>,
    forward_errors: BTreeMap<String, String>,
    forwarding_timer: slint::Timer,
}
#[derive(Default)]
struct Mailbox {
    events: VecDeque<Event>,
    screens: BTreeMap<String, Snapshot>,
    scheduled: bool,
}
thread_local! { static STATE:RefCell<Option<State>>=const{RefCell::new(None)}; }
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|state| {
        state
            .try_borrow_mut()
            .ok()
            .and_then(|mut state| state.as_mut().map(f))
    })
}
fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(rows)).into()
}
fn ui_color(rgb: u32) -> slint::Color {
    slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
fn drag_window(window: &slint::Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(RawWindowHandle::Win32(handle)) =
            window.window_handle().window_handle().map(|h| h.as_raw())
        {
            unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    handle.hwnd.get() as _,
                    0x00A1,
                    2,
                    0,
                );
            }
        }
    }
}

fn keypad_key(key: slint::winit_030::winit::keyboard::PhysicalKey) -> Option<char> {
    use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
    Some(match key {
        PhysicalKey::Code(KeyCode::Numpad0) => '0',
        PhysicalKey::Code(KeyCode::Numpad1) => '1',
        PhysicalKey::Code(KeyCode::Numpad2) => '2',
        PhysicalKey::Code(KeyCode::Numpad3) => '3',
        PhysicalKey::Code(KeyCode::Numpad4) => '4',
        PhysicalKey::Code(KeyCode::Numpad5) => '5',
        PhysicalKey::Code(KeyCode::Numpad6) => '6',
        PhysicalKey::Code(KeyCode::Numpad7) => '7',
        PhysicalKey::Code(KeyCode::Numpad8) => '8',
        PhysicalKey::Code(KeyCode::Numpad9) => '9',
        PhysicalKey::Code(KeyCode::NumpadDecimal) => '.',
        PhysicalKey::Code(KeyCode::NumpadComma) => ',',
        PhysicalKey::Code(KeyCode::NumpadAdd) => '+',
        PhysicalKey::Code(KeyCode::NumpadSubtract) => '-',
        PhysicalKey::Code(KeyCode::NumpadMultiply | KeyCode::NumpadStar) => '*',
        PhysicalKey::Code(KeyCode::NumpadDivide) => '/',
        PhysicalKey::Code(KeyCode::NumpadEqual) => '=',
        PhysicalKey::Code(KeyCode::NumpadEnter) => '\r',
        _ => return None,
    })
}

impl State {
    fn status(&self, message: impl AsRef<str>, error: bool) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_status_text(message.as_ref().into());
            ui.set_status_error(error)
        }
    }
    fn hosts(&self) {
        if let (Some(ui), Ok(store)) = (self.ui.upgrade(), self.store.lock()) {
            let mut groups = BTreeMap::<String, i32>::new();
            for p in &store.data.hosts {
                if !p.group.is_empty() {
                    *groups.entry(p.group.clone()).or_default() += 1;
                }
            }
            ui.set_groups(model(
                groups
                    .into_iter()
                    .map(|(name, count)| GroupRow {
                        name: name.into(),
                        count,
                    })
                    .collect(),
            ));
            ui.set_hosts(model(
                store
                    .data
                    .hosts
                    .iter()
                    .filter(|p| self.group_filter.is_empty() || p.group == self.group_filter)
                    .filter(|p| {
                        self.search.is_empty()
                            || format!("{} {} {} {}", p.name, p.host, p.username, p.group)
                                .to_lowercase()
                                .contains(&self.search)
                    })
                    .map(|p| HostRow {
                        id: p.id.clone().into(),
                        name: p.name.clone().into(),
                        address: format!("{}@{}:{}", p.username, p.host, p.port).into(),
                        group: p.group.clone().into(),
                        auth: if p.auth_type == "key" {
                            "SSH 私钥".into()
                        } else {
                            "密码".into()
                        },
                        has_secret: p.encrypted_secret.is_some(),
                    })
                    .collect(),
            ));
        }
    }
    fn tabs(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_tabs(model(
                self.order
                    .iter()
                    .filter_map(|id| {
                        self.sessions.get(id).map(|s| TabRow {
                            id: id.clone().into(),
                            title: s.profile.name.clone().into(),
                            status: if s.connected {
                                "connected".into()
                            } else {
                                s.status.clone().into()
                            },
                            disconnected: s.ended,
                            local: s.local,
                        })
                    })
                    .collect(),
            ));
        }
    }
    fn display(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_terminal_runs(self.render_model.clone().into());
            ui.set_terminal_visible(self.active.is_some());
            ui.set_active_session(self.active.clone().unwrap_or_default().into());
            if let Some(view) = self.active.as_ref().and_then(|id| self.sessions.get(id)) {
                ui.set_remote_connected(view.connected && !view.local);
                ui.set_local_session(view.local);
                ui.set_active_host(view.profile.name.clone().into());
                let scheme = self.settings.color_scheme;
                let rows: Vec<TerminalRun> = view
                    .snapshot
                    .runs
                    .iter()
                    .map(|r| TerminalRun {
                        text: r.text.clone().into(),
                        row: r.row as i32,
                        col: r.col as i32,
                        width: r.width as i32,
                        fg: ui_color(scheme.map_color(r.fg, r.fg_source)),
                        bg: ui_color(scheme.map_color(r.bg, r.bg_source)),
                        bold: r.bold,
                        underline: r.underline,
                    })
                    .collect();
                if self.render_model.row_count() != rows.len() {
                    self.render_model.set_vec(rows)
                } else {
                    for (index, row) in rows.into_iter().enumerate() {
                        if self.render_model.row_data(index).as_ref() != Some(&row) {
                            self.render_model.set_row_data(index, row);
                        }
                    }
                }
                ui.set_cursor_row(view.snapshot.cursor.0 as i32);
                ui.set_cursor_col(view.snapshot.cursor.1 as i32);
                ui.set_cursor_visible(view.connected && view.snapshot.cursor_visible);
                ui.set_reconnect_hint(view.ended && !view.local && !view.connected);
                let mut selected = Vec::new();
                if let Some((mut from, mut to)) = view.selection.filter(|(a, b)| a != b) {
                    if from > to {
                        std::mem::swap(&mut from, &mut to)
                    }
                    let rows = view.snapshot.rows as i32;
                    let cols = view.snapshot.cols as i32;
                    for row in from.0.max(0)..=to.0.min(rows - 1) {
                        let col = if row == from.0 { from.1.max(0) } else { 0 };
                        let end = if row == to.0 {
                            to.1.min(cols - 1)
                        } else {
                            cols - 1
                        };
                        if end >= col {
                            selected.push(SelectionRun {
                                row,
                                col,
                                width: end - col + 1,
                            })
                        }
                    }
                }
                ui.set_selection_runs(model(selected));
            } else {
                ui.set_selection_runs(model(Vec::new()));
                ui.set_remote_connected(false);
                ui.set_local_session(false);
                ui.set_active_host("".into());
                self.render_model.set_vec(Vec::new());
                ui.set_cursor_visible(false);
                ui.set_reconnect_hint(false)
            }
        }
    }
    fn save(&mut self, d: HostDraft) -> bool {
        let port = match if d.port.trim().is_empty() {
            Ok(22)
        } else {
            d.port.trim().parse::<u16>()
        } {
            Ok(p) if p > 0 => p,
            _ => {
                self.status("端口必须在 1–65535 之间", true);
                return false;
            }
        };
        let p = Profile {
            id: d.id.to_string(),
            name: d.name.to_string(),
            host: d.host.to_string(),
            port,
            username: d.username.to_string(),
            group: d.group.to_string(),
            auth_type: if d.auth_index == 1 {
                "key".into()
            } else {
                "password".into()
            },
            private_key_path: d.key_path.to_string(),
            encrypted_secret: None,
        };
        let mut secret = d.secret.to_string();
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("配置锁不可用"))
            .and_then(|mut s| s.save(p, Some(&secret), d.remember));
        secret.zeroize();
        match result {
            Ok(()) => {
                self.hosts();
                self.status("主机已保存", false);
                true
            }
            Err(e) => {
                self.status(e.to_string(), true);
                false
            }
        }
    }
    fn edit(&self, id: &str) -> HostDraft {
        let profile = self
            .store
            .lock()
            .ok()
            .and_then(|s| s.data.hosts.iter().find(|p| p.id == id).cloned());
        if let Some(p) = profile {
            HostDraft {
                id: p.id.into(),
                name: p.name.into(),
                host: p.host.into(),
                port: p.port.to_string().into(),
                username: p.username.into(),
                group: p.group.into(),
                auth_index: i32::from(p.auth_type == "key"),
                key_path: p.private_key_path.into(),
                secret: SharedString::default(),
                remember: p.encrypted_secret.is_some(),
            }
        } else {
            HostDraft {
                id: uuid::Uuid::new_v4().to_string().into(),
                port: "22".into(),
                remember: true,
                ..HostDraft::default()
            }
        }
    }
    fn connect(&mut self, host_id: &str) {
        let profile = self
            .store
            .lock()
            .ok()
            .and_then(|s| s.data.hosts.iter().find(|p| p.id == host_id).cloned());
        let Some(profile) = profile else {
            self.status("主机不存在", true);
            return;
        };
        self.connect_profile(profile);
    }
    fn connect_profile(&mut self, profile: Profile) {
        if self.sessions.len() >= 32 {
            self.status(
                "Close an unused session before opening more than 32 tabs",
                true,
            );
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.sessions.insert(
            id.clone(),
            SessionView {
                local: false,
                profile,
                transport: None,
                snapshot: Terminal::with_scrollback(
                    self.rows,
                    self.cols,
                    self.settings.scrollback_lines,
                )
                .snapshot(),
                status: "等待认证".into(),
                connected: false,
                ended: false,
                selection: None,
            },
        );
        self.order.push(id.clone());
        self.active = Some(id.clone());
        self.selecting = false;
        self.mouse_button = None;
        self.reset_files();
        self.tabs();
        self.display();
        self.credentials(id);
    }
    fn credentials(&mut self, id: String) {
        let Some(profile) = self.sessions.get(&id).map(|s| s.profile.clone()) else {
            return;
        };
        if profile.username.is_empty() {
            self.queue.push_back(Prompt::Username { id });
            self.next_prompt();
            return;
        }
        let encrypted = profile.encrypted_secret;
        if let Some(value) = encrypted {
            match store::unprotect(&value) {
                Ok(secret) => self.start(&id, secret),
                Err(e) => {
                    self.status(format!("无法读取已保存的凭据，请重新输入：{e}"), true);
                    self.queue.push_back(Prompt::Credential { id });
                    self.next_prompt()
                }
            }
        } else {
            self.queue.push_back(Prompt::Credential { id });
            self.next_prompt()
        }
    }
    fn start(&mut self, id: &str, secret: String) {
        let Some(view) = self.sessions.get_mut(id) else {
            return;
        };
        pages::logs::ssh_started(id, &view.profile);
        let known = self.store.lock().ok().and_then(|s| {
            s.data
                .known_hosts
                .get(&store::key(&view.profile.host, view.profile.port))
                .cloned()
        });
        view.status = "连接中".into();
        view.transport = Some(ssh::spawn_with_options(
            self.runtime.handle(),
            id.to_string(),
            view.profile.clone(),
            secret,
            known,
            self.events.clone(),
            ssh::ConnectOptions {
                keepalive_seconds: self.settings.keepalive_seconds,
                timeout_seconds: self.settings.connection_timeout_seconds,
                scrollback_lines: self.settings.scrollback_lines,
            },
        ));
        self.tabs();
    }
    fn open_terminal(&mut self) {
        if self.sessions.len() >= 32 || self.sessions.values().filter(|s| s.local).count() >= 8 {
            self.status(
                "Close an unused terminal before opening another local session",
                true,
            );
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        pages::logs::local_started(&id);
        let transport =
            opentermius::local::spawn(self.runtime.handle(), id.clone(), self.events.clone());
        let profile = Profile {
            id: id.clone(),
            name: "Local Terminal".into(),
            host: "localhost".into(),
            port: 22,
            username: "local".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            group: String::new(),
            encrypted_secret: None,
        };
        self.sessions.insert(
            id.clone(),
            SessionView {
                local: true,
                profile,
                transport: Some(transport),
                snapshot: Terminal::with_scrollback(
                    self.rows,
                    self.cols,
                    self.settings.scrollback_lines,
                )
                .snapshot(),
                status: "Starting".into(),
                connected: false,
                ended: false,
                selection: None,
            },
        );
        self.order.push(id.clone());
        self.active = Some(id);
        self.selecting = false;
        self.mouse_button = None;
        self.reset_files();
        self.tabs();
        self.display();
    }
    fn remote_ready(&self) -> bool {
        self.active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|s| s.connected && !s.local)
    }
    fn send(&self, command: Command) {
        if let Some(sender) = self
            .active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .and_then(|s| s.transport.as_ref())
            .map(|s| &s.commands)
        {
            if let Err(e) = sender.try_send(command) {
                self.status(format!("会话暂不可用：{e}"), true)
            }
        }
    }
    fn close(&mut self, id: &str) {
        pages::logs::closed(id);
        if let Some(view) = self.sessions.remove(id) {
            if let Some(transport) = view.transport {
                let _ = transport.commands.try_send(Command::Disconnect);
                self.runtime.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    transport.task.abort();
                });
            }
        }
        self.order.retain(|i| i != id);
        if self.active.as_deref() == Some(id) {
            self.active = self.order.last().cloned();
            self.selecting = false;
            self.mouse_button = None;
            self.reset_files();
        }
        self.queue.retain(|prompt| prompt.session_id() != id);
        if self.prompt.as_ref().is_some_and(|prompt| prompt.session_id() == id) {
            self.answer(false, String::new());
        }
        self.tabs();
        self.display();
    }
    fn reconnect(&mut self, id: &str) {
        let Some(view) = self.sessions.get(id) else {
            return;
        };
        if view.local || !view.ended {
            return;
        }
        let profile = view.profile.clone();
        let position = self.order.iter().position(|item| item == id).unwrap_or(0);
        // A fresh transport ID prevents a delayed Closed/Screen event from the
        // previous connection from changing the replacement session.
        self.close(id);
        self.connect_profile(profile);
        if let Some(new_id) = self.active.clone() {
            self.order.retain(|item| item != &new_id);
            self.order.insert(position.min(self.order.len()), new_id);
        }
        self.tabs();
        self.focus_terminal();
    }
    fn duplicate(&mut self, id: &str) {
        let Some(view) = self.sessions.get(id) else {
            return;
        };
        if view.local {
            self.open_terminal();
        } else {
            let profile = view.profile.clone();
            self.connect_profile(profile);
        }
        self.focus_terminal();
    }
    fn focus_terminal(&self) {
        if self.prompt.is_none() {
            if let Some(ui) = self.ui.upgrade() {
                ui.invoke_focus_terminal();
            }
        }
    }
    fn select(&mut self, row: i32, col: i32, extend: bool) {
        if let Some(view) = self.active.as_ref().and_then(|id| self.sessions.get_mut(id)) {
            let p = (
                row.clamp(0, view.snapshot.rows.saturating_sub(1) as i32),
                col.clamp(0, view.snapshot.cols.saturating_sub(1) as i32),
            );
            view.selection = Some(if extend {
                (view.selection.map_or(p, |selection| selection.0), p)
            } else {
                (p, p)
            });
        }
        self.display();
    }
    fn pointer(&mut self, kind: i32, button: i32, row: i32, col: i32, ctrl: bool, alt: bool, shift: bool) {
        let Some((mode, encoding, rows, cols)) = self
            .active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .map(|view| {
                (
                    view.snapshot.mouse_mode,
                    view.snapshot.mouse_encoding,
                    view.snapshot.rows,
                    view.snapshot.cols,
                )
            })
        else {
            return;
        };
        let row = row.clamp(0, rows.saturating_sub(1) as i32);
        let col = col.clamp(0, cols.saturating_sub(1) as i32);
        // Finish a drag through the same path even if Shift changes mid-drag.
        let tracking = mode != vt100::MouseProtocolMode::None
            && ((!shift && !self.selecting) || self.mouse_button.is_some());
        if !tracking {
            match kind {
                0 if button == 0 => {
                    self.selecting = true;
                    self.select(row, col, shift);
                }
                1 | 3 => {
                    let was_selecting = self.selecting;
                    self.selecting = false;
                    if was_selecting { self.end_selection(); }
                },
                2 => {
                    if self.selecting {
                        self.select(row, col, true);
                    }
                }
                _ => {}
            }
            return;
        }
        let button = match button {
            0 => terminal::MouseButton::Left,
            1 => terminal::MouseButton::Right,
            2 => terminal::MouseButton::Middle,
            _ => terminal::MouseButton::None,
        };
        let mods = terminal::MouseMods {
            ctrl,
            alt,
            shift: false,
        };
        let event = match kind {
            0 => {
                if button == terminal::MouseButton::None {
                    return;
                }
                self.mouse_button = Some(button);
                terminal::MouseEvent {
                    kind: terminal::MouseKind::Press,
                    button,
                    row: row as u16 + 1,
                    col: col as u16 + 1,
                    mods,
                }
            }
            1 | 3 => {
                let button = self.mouse_button.take().unwrap_or(button);
                if button == terminal::MouseButton::None {
                    return;
                }
                terminal::MouseEvent {
                    kind: terminal::MouseKind::Release,
                    button,
                    row: row as u16 + 1,
                    col: col as u16 + 1,
                    mods,
                }
            }
            _ => {
                let button = match self.mouse_button {
                    Some(button) => button,
                    None if mode == vt100::MouseProtocolMode::AnyMotion => {
                        terminal::MouseButton::None
                    }
                    None => return,
                };
                terminal::MouseEvent {
                    kind: terminal::MouseKind::Motion,
                    button,
                    row: row as u16 + 1,
                    col: col as u16 + 1,
                    mods,
                }
            }
        };
        if let Some(data) = terminal::encode_mouse(event, mode, encoding) {
            self.send(Command::Input(data));
        }
    }
    fn wheel(&mut self, row: i32, col: i32, delta: i32, ctrl: bool, alt: bool, shift: bool) {
        if delta == 0 {
            return;
        }
        let Some((mode, encoding, rows, cols)) = self
            .active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .map(|view| {
                (
                    view.snapshot.mouse_mode,
                    view.snapshot.mouse_encoding,
                    view.snapshot.rows,
                    view.snapshot.cols,
                )
            })
        else {
            return;
        };
        if mode == vt100::MouseProtocolMode::None || shift {
            self.send(Command::Scroll(delta));
            return;
        }
        let row = row.clamp(0, rows.saturating_sub(1) as i32) as u16 + 1;
        let col = col.clamp(0, cols.saturating_sub(1) as i32) as u16 + 1;
        let kind = if delta > 0 {
            terminal::MouseKind::WheelUp
        } else {
            terminal::MouseKind::WheelDown
        };
        let mods = terminal::MouseMods {
            ctrl,
            alt,
            shift: false,
        };
        let mut data = Vec::new();
        for _ in 0..delta.unsigned_abs().min(10) {
            if let Some(bytes) = terminal::encode_mouse(
                terminal::MouseEvent {
                    kind,
                    button: terminal::MouseButton::None,
                    row,
                    col,
                    mods,
                },
                mode,
                encoding,
            ) {
                data.extend_from_slice(&bytes);
            }
        }
        if !data.is_empty() {
            self.send(Command::Input(data));
        }
    }
    fn ime(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let data = terminal::encode_text(text);
        if data.len() <= 1024 * 1024 {
            self.send(Command::Input(data));
        }
    }
    fn next_prompt(&mut self) {
        if self.prompt.is_some() {
            return;
        }
        while let Some(prompt) = self.queue.pop_front() {
            let id = match &prompt {
                Prompt::Username { id }
                | Prompt::Credential { id }
                | Prompt::Host { id, .. }
                | Prompt::Auth { id, .. } => id,
            };
            if !self.sessions.contains_key(id) {
                continue;
            }
            if let Some(ui) = self.ui.upgrade() {
                ui.set_prompt_answer("".into());
                ui.set_prompt_is_key(false);
                ui.set_prompt_password(true);
                match &prompt {
                    Prompt::Username { .. } => {
                        ui.set_prompt_title("SSH authentication".into());
                        ui.set_prompt_message("Username".into());
                        ui.set_prompt_password(false);
                    }
                    Prompt::Credential { id } => {
                        let p = &self.sessions[id].profile;
                        let key = p.auth_type == "key";
                        ui.set_prompt_title(format!("连接 {}", p.name).into());
                        ui.set_prompt_message(if key {
                            "私钥口令（未加密的私钥可留空）".into()
                        } else {
                            format!("{}@{}:{} 的 SSH 密码", p.username, p.host, p.port).into()
                        });
                    }
                    Prompt::Host {
                        host,
                        port,
                        fingerprint,
                        ..
                    } => {
                        ui.set_prompt_title("首次连接 · 核对主机密钥".into());
                        ui.set_prompt_message(
                            format!(
                                "{host}:{port}\n\n{fingerprint}\n\n请通过可信渠道核对指纹后再信任。"
                            )
                            .into(),
                        );
                        ui.set_prompt_is_key(true);
                    }
                    Prompt::Auth { prompts, index, .. } => {
                        if *index >= prompts.len() {
                            if let Prompt::Auth { reply, .. } = prompt {
                                let _ = reply.send(Some(Vec::new()));
                            }
                            continue;
                        }
                        ui.set_prompt_title(
                            format!("服务器身份验证 {}/{}", index + 1, prompts.len()).into(),
                        );
                        ui.set_prompt_message(prompts[*index].0.clone().into());
                        ui.set_prompt_password(!prompts[*index].1);
                    }
                }
                ui.set_prompt_visible(true);
            }
            self.prompt = Some(prompt);
            break;
        }
    }
    fn answer(&mut self, accepted: bool, mut answer: String) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_prompt_visible(false);
            ui.set_prompt_answer("".into())
        }
        match self.prompt.take() {
            Some(Prompt::Username { id }) => {
                if accepted
                    && !answer.trim().is_empty()
                    && answer.len() <= 512
                    && !answer.chars().any(char::is_control)
                {
                    if let Some(view) = self.sessions.get_mut(&id) {
                        view.profile.username = answer.trim().into();
                    }
                    self.credentials(id);
                } else if let Some(view) = self.sessions.get_mut(&id) {
                    view.status = "已取消".into();
                    view.ended = true;
                    self.tabs();
                    self.display();
                }
            }
            Some(Prompt::Credential { id }) => {
                if accepted {
                    self.start(&id, std::mem::take(&mut answer))
                } else if let Some(s) = self.sessions.get_mut(&id) {
                    s.status = "已取消".into();
                    s.ended = true;
                    self.tabs();
                    self.display();
                }
            }
            Some(Prompt::Host {
                host,
                port,
                fingerprint,
                reply,
                ..
            }) => {
                let trusted = if accepted {
                    let result = self
                        .store
                        .lock()
                        .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                        .and_then(|mut s| s.trust(&host, port, &fingerprint));
                    if let Err(e) = &result {
                        self.status(e.to_string(), true)
                    }
                    result.is_ok()
                } else {
                    false
                };
                let _ = reply.send(trusted);
            }
            Some(Prompt::Auth {
                id,
                prompts,
                index,
                mut answers,
                reply,
            }) => {
                if !accepted {
                    let _ = reply.send(None);
                } else {
                    answers.push(std::mem::take(&mut answer));
                    if index + 1 < prompts.len() {
                        self.queue.push_front(Prompt::Auth {
                            id,
                            prompts,
                            index: index + 1,
                            answers,
                            reply,
                        })
                    } else {
                        let _ = reply.send(Some(answers));
                    }
                }
            }
            None => {}
        }
        answer.zeroize();
        self.next_prompt();
        self.focus_terminal();
    }
    fn event(&mut self, event: Event) {
        let id = match &event {
            Event::Status { id, .. } | Event::Screen { id, .. } | Event::HostKey { id, .. }
            | Event::AuthPrompt { id, .. } | Event::Files { id, .. } | Event::Transfer { id, .. }
            | Event::OverwriteRequired { id, .. } | Event::Closed { id } | Event::Bell { id }
            | Event::DeleteRequired { id, .. } | Event::TransferProgress { id, .. }
            | Event::TransferFinished { id, .. } => id,
        };
        if !self.sessions.contains_key(id) {
            return;
        }
        match event {
            Event::Status {
                id,
                connected,
                message,
                error,
            } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.connected = connected;
                    if connected {
                        s.ended = false;
                    }
                    s.status = if connected {
                        "已连接".into()
                    } else {
                        message.clone()
                    };
                }
                if connected {
                    self.selecting = false;
                    self.mouse_button = None;
                    if let Some(ui) = self.ui.upgrade() {
                        self.cols = ui.get_terminal_cols().clamp(2, 500) as u16;
                        self.rows = ui.get_terminal_rows().clamp(2, 300) as u16;
                    }
                    if let Some(transport) =
                        self.sessions.get(&id).and_then(|s| s.transport.as_ref())
                    {
                        let _ = transport.commands.try_send(Command::Resize {
                            cols: self.cols,
                            rows: self.rows,
                        });
                    }
                }
                if connected {
                    pages::logs::connected(&id);
                } else if error {
                    pages::logs::failed(&id, &message);
                }
                self.status(
                    if connected {
                        "Connected".to_string()
                    } else {
                        message
                    },
                    error,
                );
                self.tabs();
                self.display();
                if connected && self.pending_sftp.as_deref() == Some(&id) {
                    self.pending_sftp = None;
                    if self.ui.upgrade().is_some_and(|ui| ui.get_sftp_page()) {
                        self.active = Some(id.clone());
                        self.display();
                        self.list_files(".".into());
                    }
                }
            }
            Event::Screen { id, snapshot } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if s.snapshot.lines != snapshot.lines {
                        s.selection = None;
                    }
                    if snapshot.bell_count != s.snapshot.bell_count
                        && self.settings.bell == BellMode::Visual
                        && self.active.as_deref() == Some(&id)
                    {
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_bell_flash(true);
                        }
                    }
                    s.snapshot = snapshot;
                    if self.active.as_deref() == Some(&id) {
                        self.display();
                    }
                }
            }
            Event::HostKey {
                id,
                host,
                port,
                fingerprint,
                reply,
            } => {
                self.queue.push_back(Prompt::Host {
                    id,
                    host,
                    port,
                    fingerprint,
                    reply,
                });
                self.next_prompt()
            }
            Event::AuthPrompt { id, prompts, reply } => {
                self.queue.push_back(Prompt::Auth {
                    id,
                    prompts,
                    index: 0,
                    answers: Vec::new(),
                    reply,
                });
                self.next_prompt()
            }
            Event::Files { id, path, entries } => {
                if self.active.as_deref() == Some(&id) {
                    self.file_path = path.clone();
                    self.remote_entries = entries;
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_file_path(path.into());
                        ui.set_remote_selection("".into());
                        self.render_files();
                        ui.set_file_busy(false)
                    }
                }
            }
            Event::Transfer { id, message, error } => {
                if self.active.as_deref() == Some(&id) {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_file_busy(false)
                    }
                    self.status(message, error)
                }
            }
            Event::OverwriteRequired {
                id,
                directory,
                local,
                transfer,
                folder,
            } => {
                let confirmed = rfd::MessageDialog::new()
                    .set_title("覆盖远程文件")
                    .set_description(if folder {
                        "同名文件夹已存在，是否合并并覆盖其中的同名文件？"
                    } else {
                        "同名文件已存在，是否覆盖？"
                    })
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if confirmed {
                    if let Some(transport) = self.sessions.get(&id).and_then(|s| s.transport.as_ref()) {
                        let command = if transfer == 0 {
                            Command::Upload {
                                directory,
                                local,
                                overwrite: true,
                            }
                        } else if folder {
                            Command::UploadTree {
                                transfer,
                                directory,
                                local,
                                overwrite: true,
                            }
                        } else {
                            Command::UploadFile {
                                transfer,
                                directory,
                                local,
                                overwrite: true,
                            }
                        };
                        if transport.commands.try_send(command).is_err() {
                            self.finish_transfer(transfer, "传输队列繁忙".into(), true, false);
                        }
                    }
                } else {
                    self.finish_transfer(transfer, "传输已取消".into(), false, true);
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_file_busy(false)
                    }
                }
            }
            Event::Bell { id } => {
                if self.settings.bell == BellMode::Visual && self.active.as_deref() == Some(&id) {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_bell_flash(true);
                    }
                }
            }
            Event::DeleteRequired {
                id,
                directory,
                name,
                files,
                dirs,
                bytes,
            } => {
                let confirmed = rfd::MessageDialog::new()
                    .set_title("确认删除")
                    .set_description(format!(
                        "删除 {name}？\n{files} 个文件，{dirs} 个子文件夹，{}。此操作无法撤销。",
                        format_size(bytes)
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if confirmed {
                    if let Some(transport) = self.sessions.get(&id).and_then(|s| s.transport.as_ref()) {
                        if transport
                            .commands
                            .try_send(Command::Delete {
                                directory,
                                name,
                                confirmed: true,
                            })
                            .is_err()
                        {
                            self.status("文件任务队列繁忙", true);
                        }
                    }
                }
            }
            Event::TransferProgress {
                id,
                transfer,
                name,
                direction,
                transferred,
                total,
                speed,
            } => {
                if let Some((_, row)) = self
                    .transfers
                    .get_mut(&transfer)
                    .filter(|(session, _)| session == &id)
                {
                    row.name = format!("{direction} · {name}").into();
                    row.percent = if total == 0 {
                        0
                    } else {
                        (transferred.saturating_mul(100) / total).min(100) as i32
                    };
                    row.detail = format!(
                        "{} / {} · {}% · {}/s",
                        format_size(transferred),
                        format_size(total),
                        row.percent,
                        format_size(speed)
                    )
                    .into();
                    self.render_transfers();
                }
            }
            Event::TransferFinished {
                id,
                transfer,
                message,
                error,
                cancelled,
            } => {
                if self
                    .transfers
                    .get(&transfer)
                    .is_some_and(|(session, _)| session == &id)
                {
                    self.finish_transfer(transfer, message, error, cancelled);
                    if self.active.as_deref() == Some(&id) && !error && !cancelled {
                        self.list_files(self.file_path.clone());
                        self.list_local(String::new());
                    }
                }
            }
            Event::Closed { id } => {
                pages::logs::closed(&id);
                for (session, row) in self.transfers.values_mut() {
                    if session == &id && row.pending {
                        row.pending = false;
                        row.detail = "连接已关闭，传输已取消".into();
                    }
                }
                self.render_transfers();
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.connected = false;
                    s.ended = true;
                    if s.status == "已连接" || s.status == "连接中" {
                        s.status = "已断开".into();
                    }
                }
                if self.active.as_deref() == Some(&id) {
                    self.selecting = false;
                    self.mouse_button = None;
                }
                self.queue.retain(|prompt| match prompt {
                    Prompt::Username { id: pending }
                    | Prompt::Credential { id: pending }
                    | Prompt::Host { id: pending, .. }
                    | Prompt::Auth { id: pending, .. } => pending != &id,
                });
                let current = self.prompt.as_ref().is_some_and(|prompt| match prompt {
                    Prompt::Username { id: pending }
                    | Prompt::Credential { id: pending }
                    | Prompt::Host { id: pending, .. }
                    | Prompt::Auth { id: pending, .. } => pending == &id,
                });
                if current {
                    self.answer(false, String::new())
                }
                self.tabs();
                self.display();
            }
        }
    }
    fn key(&mut self, text: &str, ctrl: bool, alt: bool, shift: bool) {
        if ctrl && shift && text.eq_ignore_ascii_case("c") {
            self.copy();
            return;
        }
        if ctrl && shift && text.eq_ignore_ascii_case("v") {
            self.paste();
            return;
        }
        let Some(id) = self.active.clone() else {
            return;
        };
        let Some((ended, local, app_cursor)) = self
            .sessions
            .get(&id)
            .map(|view| (view.ended, view.local, view.snapshot.application_cursor))
        else {
            return;
        };
        if ended {
            if !local
                && !ctrl
                && !alt
                && matches!(
                    terminal::Key::from_text(text),
                    Some(terminal::Key::Named(terminal::NamedKey::Enter))
                )
            {
                self.reconnect(&id);
            }
            return;
        }
        let Some(key) = terminal::Key::from_text(text) else {
            return;
        };
        let mods = terminal::KeyMods::new(ctrl, alt, shift);
        if let Some(data) = terminal::encode_key(key, mods, app_cursor) {
            self.send(Command::Input(data));
        }
    }
    fn keypad(&self, key: char, mods: terminal::KeyMods) -> bool {
        let application = self.active.as_ref().and_then(|id| self.sessions.get(id))
            .is_some_and(|view| !view.ended && view.snapshot.application_keypad);
        if application {
            if let Some(data) = terminal::encode_keypad(key, mods, true) {
                self.send(Command::Input(data));
                return true;
            }
        }
        false
    }
    fn copy(&self) {
        if let Some(s) = self.active.as_ref().and_then(|id| self.sessions.get(id)) {
            let text = if let Some((start, end)) = s.selection.filter(|(a, b)| a != b) {
                s.snapshot.selected_text(start, end)
            } else {
                s.snapshot.lines.join("\n").trim_end().to_string()
            };
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Err(e) = clipboard.set_text(text) {
                    self.status(e.to_string(), true)
                }
            }
        }
    }
    fn end_selection(&self) {
        if !self.settings.copy_on_select {
            return;
        }
        let Some(view) = self.active.as_ref().and_then(|id| self.sessions.get(id)) else {
            return;
        };
        let Some((start, end)) = view.selection.filter(|(a, b)| a != b) else {
            return;
        };
        let text = view.snapshot.selected_text(start, end);
        if text.is_empty() {
            return;
        }
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Err(e) = clipboard.set_text(text) {
                self.status(e.to_string(), true)
            }
        }
    }
    fn paste(&self) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Ok(text) = clipboard.get_text() {
                if text.len() > 1024 * 1024 {
                    self.status("粘贴内容不能超过 1 MB", true);
                    return;
                }
                let lines = opentermius::settings::paste_line_count(&text);
                if self.settings.confirm_multiline_paste && lines > 1 {
                    let confirmed = rfd::MessageDialog::new()
                        .set_title("粘贴多行内容")
                        .set_description(format!("剪贴板包含 {lines} 行内容，是否粘贴到终端？"))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        == rfd::MessageDialogResult::Yes;
                    if !confirmed {
                        self.status("已取消粘贴", false);
                        return;
                    }
                }
                let bracketed = self
                    .active
                    .as_ref()
                    .and_then(|id| self.sessions.get(id))
                    .is_some_and(|s| s.snapshot.bracketed_paste);
                let data = if bracketed {
                    format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
                } else {
                    text.replace("\r\n", "\r").replace('\n', "\r")
                };
                self.send(Command::Input(data.into_bytes()));
            }
        }
    }
    fn list_files(&self, path: String) {
        if !self.remote_ready() {
            self.status("请先选择已连接的远程主机", true);
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_files_open(true);
            ui.set_file_busy(true)
        }
        self.send(Command::List(path));
    }
    fn reset_files(&mut self) {
        self.file_path = ".".into();
        self.remote_entries.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_files(model(Vec::new()));
            ui.set_file_path(".".into());
            ui.set_remote_selection("".into());
            ui.set_file_busy(false)
        }
    }
    fn list_local(&mut self, value: String) {
        let path = if value.is_empty() {
            self.local_path.clone()
        } else {
            PathBuf::from(value)
        };
        self.local_request += 1;
        let generation = self.local_request;
        let show_hidden = self.settings.show_hidden_files;
        let weak = self.ui.clone();
        self.runtime.spawn_blocking(move || {
            let result = (|| -> anyhow::Result<(PathBuf, Vec<ssh::LocalFile>)> {
                let path = path.canonicalize()?;
                let mut files = ssh::read_local_dir(&path)?;
                if !show_hidden {
                    files.retain(|file| {
                        if ssh::is_hidden_name(&file.name) { return false; }
                        #[cfg(windows)]
                        {
                            use std::os::windows::fs::MetadataExt;
                            if file.path.metadata().is_ok_and(|meta| meta.file_attributes() & 0x2 != 0) {
                                return false;
                            }
                        }
                        true
                    });
                }
                Ok((path, files))
            })();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                with_state(|s| {
                    if s.local_request != generation {
                        return;
                    }
                    match result {
                        Ok((path, files)) => {
                            s.local_path = path;
                            s.local_entries = files;
                            let display = s.local_path.to_string_lossy();
                            ui.set_local_path(
                                display.strip_prefix(r"\\?\").unwrap_or(&display).into(),
                            );
                            ui.set_local_selection("".into());
                            s.render_files();
                        }
                        Err(e) => s.status(e.to_string(), true),
                    }
                });
            });
        });
    }
    fn render_files(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let hidden = ui.get_show_hidden();
        ssh::sort_remote_files(
            &mut self.remote_entries,
            ssh::SortColumn::parse(&ui.get_remote_sort()),
            ui.get_remote_ascending(),
        );
        ssh::sort_local_files(
            &mut self.local_entries,
            ssh::SortColumn::parse(&ui.get_local_sort()),
            ui.get_local_ascending(),
        );
        ui.set_files(model(
            self.remote_entries
                .iter()
                .filter(|e| hidden || !ssh::is_hidden_name(&e.name))
                .map(|e| {
                    file_row(
                        &e.name,
                        e.is_dir,
                        e.size,
                        e.modified.parse().unwrap_or(0),
                        e.permissions,
                    )
                })
                .collect(),
        ));
        ui.set_local_files(model(
            self.local_entries
                .iter()
                .filter(|e| hidden || !ssh::is_hidden_name(&e.name))
                .map(|e| file_row(&e.name, e.is_dir, e.size, e.modified, None))
                .collect(),
        ));
    }
    fn render_transfers(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_transfers(model(
                self.transfers
                    .values()
                    .filter(|(_, row)| row.pending)
                    .chain(self.transfers.values().rev().filter(|(_, row)| !row.pending))
                    .map(|(_, row)| row.clone())
                    .collect(),
            ));
        }
    }
    fn finish_transfer(&mut self, transfer: u64, message: String, error: bool, cancelled: bool) {
        if let Some((_, row)) = self.transfers.get_mut(&transfer) {
            row.pending = false;
            if !error && !cancelled {
                row.percent = 100;
            }
            row.detail = message.clone().into();
        }
        self.render_transfers();
        self.status(message, error);
    }
    fn queue_transfer(&mut self, name: &str, direction: &str, job: impl FnOnce(u64) -> Command) {
        if !self.remote_ready() {
            self.status("请先选择已连接的远程主机", true);
            return;
        }
        if self
            .transfers
            .values()
            .filter(|(_, row)| row.pending)
            .count()
            >= 8
        {
            self.status("传输队列已满（最多 8 项），请等待或取消任务", true);
            return;
        }
        while self.transfers.len() >= 32 {
            let old = self
                .transfers
                .iter()
                .find(|(_, (_, row))| !row.pending)
                .map(|(id, _)| *id);
            if let Some(old) = old {
                self.transfers.remove(&old);
            } else {
                break;
            }
        }
        self.next_transfer += 1;
        let transfer = self.next_transfer;
        let id = self.active.clone().unwrap();
        let row = TransferRow {
            id: transfer.to_string().into(),
            name: format!("{direction} · {name}").into(),
            detail: "排队中".into(),
            percent: 0,
            pending: true,
        };
        self.transfers.insert(transfer, (id.clone(), row));
        let result = self.sessions[&id]
            .transport
            .as_ref()
            .unwrap()
            .commands
            .try_send(job(transfer));
        if result.is_err() {
            self.finish_transfer(transfer, "传输队列繁忙".into(), true, false);
        }
        self.render_transfers();
    }
    fn upload_path(&mut self, local: PathBuf) {
        let name = local
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let directory = self.file_path.clone();
        let folder = local.is_dir();
        let overwrite = !self.settings.confirm_overwrite;
        self.queue_transfer(&name, "Upload", |transfer| {
            if folder {
                Command::UploadTree {
                    transfer,
                    directory,
                    local,
                    overwrite,
                }
            } else {
                Command::UploadFile {
                    transfer,
                    directory,
                    local,
                    overwrite,
                }
            }
        });
    }
    fn download_entry(&mut self, name: &str, destination: Option<PathBuf>) {
        if !ssh::safe_local_component(name) {
            self.status("远程文件名无法用于本地路径", true);
            return;
        }
        let Some(entry) = self.remote_entries.iter().find(|e| e.name == name) else {
            return;
        };
        let remote = entry.path.clone();
        let folder = entry.is_dir;
        let local = destination.unwrap_or_else(|| {
            if folder {
                self.local_path.clone()
            } else {
                self.local_path.join(name)
            }
        });
        self.queue_transfer(name, "Download", |transfer| {
            if folder {
                Command::DownloadTree {
                    transfer,
                    remote,
                    local_dir: local,
                }
            } else {
                Command::DownloadFile {
                    transfer,
                    remote,
                    local,
                }
            }
        });
    }
    fn file_operation(&mut self, local: bool, operation: &str, name: &str, value: &str) {
        let new_name = operation == "mkdir" || operation == "rename";
        if (operation != "mkdir"
            && !(if local {
                ssh::safe_local_component(name)
            } else {
                ssh::safe_remote_component(name)
            }))
            || (new_name
                && !(if local {
                    ssh::safe_local_component(value)
                } else {
                    ssh::safe_remote_component(value)
                }))
        {
            self.status("文件名无效", true);
            return;
        }
        if !local {
            if !self.remote_ready() {
                self.status("请先连接远程主机", true);
                return;
            }
            let directory = self.file_path.clone();
            match operation {
                "mkdir" => self.send(Command::CreateDir {
                    directory,
                    name: value.into(),
                }),
                "rename" => self.send(Command::Rename {
                    directory,
                    from: name.into(),
                    to: value.into(),
                }),
                "delete" => self.send(Command::Delete {
                    directory,
                    name: name.into(),
                    confirmed: false,
                }),
                _ => {}
            }
            return;
        }
        let root = self.local_path.clone();
        let path = root.join(name);
        let destination = root.join(value);
        let operation = operation.to_string();
        let weak = self.ui.clone();
        self.runtime.spawn_blocking(move || {
            if operation == "delete" {
                let summary = ssh::local_tree_summary(&path);
                let _ = weak.upgrade_in_event_loop(move |_| {
                    let summary = match summary {
                        Ok(summary) => summary,
                        Err(e) => {
                            with_state(|s| s.status(e.to_string(), true));
                            return;
                        }
                    };
                    let confirmed = rfd::MessageDialog::new()
                        .set_title("确认删除")
                        .set_description(format!(
                            "删除 {}？\n{} 个文件，{} 个子文件夹，{}。此操作无法撤销。",
                            path.display(),
                            summary.files,
                            summary.dirs,
                            format_size(summary.bytes)
                        ))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        == rfd::MessageDialogResult::Yes;
                    if confirmed {
                        with_state(|s| {
                            let weak = s.ui.clone();
                            s.runtime.spawn_blocking(move || {
                                let result = ssh::delete_local_tree(&path);
                                let _ = weak.upgrade_in_event_loop(move |_| {
                                    with_state(|s| {
                                        match result {
                                            Ok(()) => s.status("已删除", false),
                                            Err(e) => s.status(e.to_string(), true),
                                        };
                                        s.list_local(String::new());
                                    });
                                });
                            });
                        });
                    }
                });
            } else {
                let result = (|| -> anyhow::Result<()> {
                    anyhow::ensure!(!destination.try_exists()?, "目标名称已存在");
                    match operation.as_str() {
                        "mkdir" => std::fs::create_dir(destination)?,
                        "rename" => std::fs::rename(path, destination)?,
                        _ => anyhow::bail!("不支持的文件操作"),
                    }
                    Ok(())
                })();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    with_state(|s| {
                        match result {
                            Ok(()) => s.status("文件操作已完成", false),
                            Err(e) => s.status(e.to_string(), true),
                        };
                        s.list_local(String::new());
                    });
                });
            }
        });
    }
    fn select_sftp(&mut self, host_id: &str) {
        if let Some(id) = self
            .order
            .iter()
            .find(|id| {
                self.sessions
                    .get(*id)
                    .is_some_and(|s| s.profile.id == host_id && s.connected && !s.local)
            })
            .cloned()
        {
            self.active = Some(id);
            self.reset_files();
            self.display();
            self.list_files(".".into());
            return;
        }
        self.connect(host_id);
        self.pending_sftp = self.active.clone();
    }
}
fn format_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.)
    } else {
        format!("{:.1} MB", size as f64 / 1048576.)
    }
}
fn safe_filename(name: &str) -> bool {
    ssh::safe_local_component(name)
}
fn file_row(
    name: &str,
    is_dir: bool,
    size: u64,
    modified: i64,
    permissions: Option<u32>,
) -> FileRow {
    FileRow {
        name: name.into(),
        is_directory: is_dir,
        size: if is_dir {
            "".into()
        } else {
            format_size(size).into()
        },
        modified: chrono::DateTime::from_timestamp(modified, 0)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default()
            .into(),
        kind: if is_dir {
            "Folder".into()
        } else {
            std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("File")
                .into()
        },
        permissions: permissions
            .map(ssh::format_permissions)
            .unwrap_or_default()
            .into(),
    }
}
fn capture(ui: &AppWindow, output: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(output)?;
    let image = ui.window().take_snapshot()?;
    image::save_buffer(
        output.join("native-window.png"),
        image.as_bytes(),
        image.width(),
        image.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}

fn run() -> anyhow::Result<()> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let ui = AppWindow::new()?;
    let args: Vec<String> = std::env::args().collect();
    let smoke = args.iter().any(|a| a == "--smoke-test");
    let benchmark = args.iter().any(|a| a == "--benchmark");
    let scene = args
        .iter()
        .find_map(|a| a.strip_prefix("--smoke-scene="))
        .unwrap_or("empty")
        .to_string();
    let smoke_port = args
        .iter()
        .find_map(|a| a.strip_prefix("--smoke-port="))
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|p| *p > 0);
    anyhow::ensure!(
        !smoke
            || (scene == "empty" && smoke_port.is_none())
            || args.iter().any(|a| a.starts_with("--data-dir=")),
        "写入自检资料时必须指定独立 --data-dir"
    );
    let isolated_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--data-dir="))
        .map(|p| PathBuf::from(p).join("opentermius-native.json"));
    let store = Arc::new(Mutex::new(match isolated_path {
        Some(path) => Store::open(path)?,
        None => store::open_default()?,
    }));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .enable_all()
        .build()?;
    let (tx, mut rx) = mpsc::channel::<Event>(128);
    let weak = ui.as_weak();
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    runtime.spawn(async move {
        while let Some(event) = rx.recv().await {
            let schedule = {
                let mut pending = mailbox.lock().unwrap();
                match event {
                    Event::Screen { id, snapshot } => {
                        pending.screens.insert(id, snapshot);
                    }
                    event => pending.events.push_back(event),
                };
                if pending.scheduled {
                    false
                } else {
                    pending.scheduled = true;
                    true
                }
            };
            if schedule {
                let pending = mailbox.clone();
                if weak
                    .upgrade_in_event_loop(move |_| {
                        let (events, screens) = {
                            let mut pending = pending.lock().unwrap();
                            pending.scheduled = false;
                            (
                                std::mem::take(&mut pending.events),
                                std::mem::take(&mut pending.screens),
                            )
                        };
                        with_state(|state| {
                            for event in events {
                                state.event(event)
                            }
                            for (id, snapshot) in screens {
                                state.event(Event::Screen { id, snapshot })
                            }
                        });
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let initial_settings = store
        .lock()
        .map(|s| s.data.settings.clone().normalized())
        .unwrap_or_default();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            ui: ui.as_weak(),
            store,
            settings: initial_settings,
            runtime,
            events: tx,
            sessions: BTreeMap::new(),
            order: Vec::new(),
            active: None,
            queue: VecDeque::new(),
            prompt: None,
            cols: 110,
            rows: 32,
            file_path: ".".into(),
            search: String::new(),
            group_filter: String::new(),
            local_path: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            local_request: 0,
            pending_sftp: None,
            remote_entries: Vec::new(),
            local_entries: Vec::new(),
            transfers: BTreeMap::new(),
            next_transfer: 0,
            render_model: Rc::new(VecModel::from(Vec::new())),
            selecting: false,
            mouse_button: None,
            forwards: BTreeMap::new(),
            forward_errors: BTreeMap::new(),
            forwarding_timer: slint::Timer::default(),
        })
    });
    ui.on_save_host(|draft| with_state(|s| s.save(draft)).unwrap_or(false));
    ui.on_save_and_connect(|draft| {
        with_state(|s| {
            let mut id = draft.id.to_string();
            if s.save(draft) {
                if id.is_empty() {
                    id = s
                        .store
                        .lock()
                        .ok()
                        .and_then(|s| s.data.hosts.last().map(|p| p.id.clone()))
                        .unwrap_or_default()
                }
                if let Some(ui) = s.ui.upgrade() {
                    ui.invoke_close_editor()
                }
                s.connect(&id)
            }
        });
    });
    ui.on_quick_connect(|value| {
        with_state(|s| match store::parse_quick_connect(&value) {
            Ok(profile) => s.connect_profile(profile),
            Err(e) => s.status(e.to_string(), true),
        });
    });
    ui.on_open_terminal(|| {
        with_state(|s| s.open_terminal());
    });
    let weak = ui.as_weak();
    ui.on_window_minimize(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_minimized(true)
        }
    });
    let weak = ui.as_weak();
    ui.on_window_maximize(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_maximized(!ui.window().is_maximized())
        }
    });
    ui.on_window_close(|| {
        let _ = slint::quit_event_loop();
    });
    let weak = ui.as_weak();
    ui.on_window_drag(move || {
        if let Some(ui) = weak.upgrade() {
            drag_window(ui.window());
        }
    });
    ui.on_search_hosts(|query| {
        with_state(|s| {
            s.search = query.to_lowercase();
            s.hosts()
        });
    });
    ui.on_select_group(|name| {
        with_state(|s| {
            s.group_filter = name.to_string();
            s.hosts();
        });
    });
    ui.on_edit_host(|id| with_state(|s| s.edit(&id)).unwrap_or_default());
    ui.on_delete_host(|id| {
        with_state(|s| {
            let result = s
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                .and_then(|mut store| store.delete(&id));
            match result {
                Ok(()) => s.hosts(),
                Err(e) => s.status(e.to_string(), true),
            }
        });
    });
    ui.on_connect_host(|id| {
        with_state(|s| s.connect(&id));
    });
    ui.on_select_session(|id| {
        with_state(|s| {
            s.selecting = false;
            s.mouse_button = None;
            s.active = if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            };
            s.reset_files();
            s.display();
            if s.remote_ready()
                && s.ui
                    .upgrade()
                    .is_some_and(|ui| ui.get_sftp_page() || ui.get_files_open())
            {
                s.list_files(".".into());
            }
        });
    });
    ui.on_close_session(|id| {
        with_state(|s| s.close(&id));
    });
    ui.on_answer_prompt(|accepted, answer| {
        with_state(|s| s.answer(accepted, answer.to_string()));
    });
    ui.on_terminal_key(|text, ctrl, alt, shift| {
        with_state(|s| s.key(&text, ctrl, alt, shift));
    });
    // Public Slint key events omit the physical key location. Handle the
    // application keypad before normalization, including NumLock-off keys.
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
        let weak = ui.as_weak();
        let mut modifiers = winit::keyboard::ModifiersState::empty();
        ui.window().on_winit_window_event(move |_, event| {
            if let winit::event::WindowEvent::ModifiersChanged(value) = event {
                modifiers = value.state();
            }
            if let winit::event::WindowEvent::KeyboardInput { event, is_synthetic: false, .. } = event {
                if event.state == winit::event::ElementState::Pressed
                    && weak.upgrade().is_some_and(|ui| ui.get_terminal_has_focus())
                {
                    if let Some(key) = keypad_key(event.physical_key) {
                        let mods = terminal::KeyMods::new(modifiers.control_key(), modifiers.alt_key(), modifiers.shift_key());
                        if with_state(|s| s.keypad(key, mods)).unwrap_or(false) {
                            return EventResult::PreventDefault;
                        }
                    }
                }
            }
            EventResult::Propagate
        });
    }
    ui.on_terminal_size(|cols, rows| {
        with_state(|s| {
            s.cols = cols.clamp(2, 500) as u16;
            s.rows = rows.clamp(2, 300) as u16;
            s.send(Command::Resize {
                cols: s.cols,
                rows: s.rows,
            });
        });
    });
    ui.on_terminal_scroll(|delta| {
        with_state(|s| s.send(Command::Scroll(delta)));
    });
    ui.on_terminal_pointer(|kind, button, row, col, ctrl, alt, shift| {
        with_state(|s| s.pointer(kind, button, row, col, ctrl, alt, shift));
    });
    ui.on_terminal_wheel(|row, col, delta, ctrl, alt, shift| {
        with_state(|s| s.wheel(row, col, delta, ctrl, alt, shift));
    });
    ui.on_terminal_ime(|text| {
        with_state(|s| s.ime(&text));
    });
    ui.on_reconnect_session(|id| {
        with_state(|s| s.reconnect(&id));
    });
    ui.on_duplicate_session(|id| {
        with_state(|s| s.duplicate(&id));
    });
    ui.on_select_terminal(|row, col, extend| {
        with_state(|s| s.select(row, col, extend));
    });
    ui.on_copy_terminal(|| {
        with_state(|s| s.copy());
    });
    ui.on_end_terminal_selection(|| {
        with_state(|s| s.end_selection());
    });
    ui.on_paste_terminal(|| {
        with_state(|s| s.paste());
    });
    ui.on_pick_key(|| {
        rfd::FileDialog::new()
            .set_title("选择 SSH 私钥")
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned().into())
            .unwrap_or_default()
    });
    ui.on_list_files(|path| {
        with_state(|s| s.list_files(path.to_string()));
    });
    ui.on_open_sftp(|| {
        with_state(|s| {
            s.search.clear();
            s.group_filter.clear();
            s.hosts();
            s.list_local(String::new());
            if let Some(ui) = s.ui.upgrade() {
                ui.set_sftp_page(true)
            }
            if s.remote_ready() {
                s.list_files(".".into())
            }
        });
    });
    ui.on_list_local(|path| {
        with_state(|s| s.list_local(path.to_string()));
    });
    ui.on_select_sftp_host(|id| {
        with_state(|s| s.select_sftp(&id));
    });
    ui.on_upload_local(|name| {
        with_state(|s| {
            if !safe_filename(&name) {
                s.status("本地文件名无效", true);
                return;
            }
            if !s.remote_ready() {
                s.status("请先选择远程主机", true);
                return;
            }
            s.upload_path(s.local_path.join(name.as_str()));
        });
    });
    ui.on_download_to_local(|name| {
        with_state(|s| {
            s.download_entry(&name, None);
        });
    });
    ui.on_upload_file(|| {
        if let Some(local) = rfd::FileDialog::new().set_title("上传文件").pick_file() {
            with_state(|s| {
                s.upload_path(local);
            });
        }
    });
    ui.on_download_file(|name| {
        if let Some(local) = rfd::FileDialog::new()
            .set_title("保存下载文件")
            .set_file_name(name.as_str())
            .save_file()
        {
            with_state(|s| {
                s.download_entry(&name, Some(local));
            });
        }
    });
    ui.on_upload_folder(|| {
        if let Some(local) = rfd::FileDialog::new().set_title("上传文件夹").pick_folder() {
            with_state(|s| s.upload_path(local));
        }
    });
    ui.on_file_operation(|local, operation, name, value| {
        with_state(|s| s.file_operation(local, &operation, &name, &value));
    });
    ui.on_update_file_view(|| {
        with_state(|s| s.render_files());
    });
    ui.on_cancel_transfer(|value| {
        with_state(|s| {
            let Ok(transfer) = value.parse::<u64>() else {
                return;
            };
            if let Some((id, row)) = s
                .transfers
                .get_mut(&transfer)
                .filter(|(_, row)| row.pending)
            {
                if let Some(transport) = s
                    .sessions
                    .get(id)
                    .and_then(|session| session.transport.as_ref())
                {
                    if transport
                        .commands
                        .try_send(Command::CancelTransfer { transfer })
                        .is_ok()
                    {
                        row.detail = "正在取消…".into();
                    } else {
                        s.status("取消请求未发送，请重试", true);
                    }
                }
            }
            s.render_transfers();
        });
    });
    ui.on_import_hosts(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("导入主机")
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            with_state(|s| {
                let result = s
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                    .and_then(|mut store| store.import(&path));
                match result {
                    Ok(n) => {
                        s.hosts();
                        s.status(format!("已导入 {n} 台主机"), false)
                    }
                    Err(e) => s.status(e.to_string(), true),
                }
            });
        }
    });
    ui.on_import_xshell(|| {
        let default = opentermius::import_xshell::default_sessions_dir();
        let mut picker = rfd::FileDialog::new().set_title("Import from Xshell — Select Sessions folder");
        if let Some(path) = &default {
            picker = picker.set_directory(path);
        }
        let choice = default.as_ref().map(|path| {
            rfd::MessageDialog::new()
                .set_title("Import from Xshell")
                .set_description(format!("Import sessions from {}?\nChoose No to select another folder.", path.display()))
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show()
        });
        let path = match choice {
            Some(rfd::MessageDialogResult::Yes) => default,
            Some(rfd::MessageDialogResult::Cancel) => return,
            _ => picker.pick_folder(),
        };
        if let Some(path) = path {
            let scanned = opentermius::import_xshell::scan(&path);
            let message = with_state(|s| {
                let result = s.store.lock()
                    .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                    .and_then(|mut store| opentermius::import_xshell::merge(&mut store, scanned));
                match result {
                    Ok(summary) => {
                        s.hosts();
                        let message = summary.message();
                        s.status(&message, false);
                        Some(message)
                    }
                    Err(error) => {
                        s.status(error.to_string(), true);
                        None
                    }
                }
            }).flatten();
            if let Some(message) = message {
                rfd::MessageDialog::new()
                    .set_title("Import from Xshell")
                    .set_description(message)
                    .show();
            }
        }
    });
    ui.on_export_hosts(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("导出主机（不含凭据）")
            .add_filter("JSON", &["json"])
            .set_file_name("opentermius-hosts.json")
            .save_file()
        {
            with_state(|s| {
                let result = s
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                    .and_then(|store| store.export(&path));
                s.status(
                    result
                        .map(|_| "已导出主机配置（不含凭据）".to_string())
                        .unwrap_or_else(|e| e.to_string()),
                    false,
                )
            });
        }
    });
    pages::install(&ui);
    with_state(|s| {
        s.hosts();
        s.tabs();
        s.display()
    });
    if smoke && (scene == "hosts" || smoke_port.is_some()) {
        anyhow::ensure!(
            ui.invoke_save_host(HostDraft {
                name: "本机验证服务器".into(),
                host: "127.0.0.1".into(),
                port: smoke_port.unwrap_or(22).to_string().into(),
                username: "tester".into(),
                group: "开发环境".into(),
                ..HostDraft::default()
            }),
            "界面创建主机失败"
        );
        anyhow::ensure!(ui.get_hosts().row_count() == 1, "主机列表未更新");
        if smoke_port.is_some() {
            let id = ui.get_hosts().row_data(0).unwrap().id;
            ui.invoke_connect_host(id)
        }
    }
    if smoke && scene == "editor" {
        ui.invoke_new_host();
    }
    if smoke && (scene == "section-terminal" || scene == "section-terminal-menu") {
        // Deterministic disconnected SSH fixture; no network or saved secrets.
        with_state(|s| {
            let mut terminal = Terminal::new(s.rows, s.cols);
            terminal.feed("\x1b[32mdev@workspace\x1b[0m:~$ echo 'Terminal ready · 中文输入'\r\nTerminal ready · 中文输入\r\n\r\n\x1b[33mSSH session ended.\x1b[0m\r\n".as_bytes());
            let id = "terminal-smoke".to_string();
            s.sessions.insert(id.clone(), SessionView {
                local: false,
                profile: Profile {
                    id: "smoke-profile".into(), name: "Development".into(), host: "localhost".into(),
                    port: 22, username: "dev".into(), auth_type: "password".into(),
                    private_key_path: String::new(), group: String::new(), encrypted_secret: None,
                },
                transport: None, snapshot: terminal.snapshot(), status: "已断开".into(),
                connected: false, ended: true, selection: None,
            });
            s.order.push(id.clone());
            s.active = Some(id);
            s.tabs();
            s.display();
        });
        ui.invoke_focus_terminal();
        if scene == "section-terminal-menu" {
            ui.set_tab_menu_session("terminal-smoke".into());
            ui.set_tab_menu_reconnectable(true);
            ui.set_tab_menu_x(410.0);
        }
    } else if let Some(section) = scene.strip_prefix("section-").filter(|_| smoke) {
        if section == "xshell-import" {
            ui.invoke_open_section("hosts".into());
            ui.set_menu_open(true);
        } else {
            ui.invoke_open_section(section.into());
        }
    }
    if smoke && (scene == "sftp" || scene == "section-sftp") {
        with_state(|s| {
            let folder = s.store.lock().unwrap().path.parent().unwrap().join("files");
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(
                folder.join("readme.txt"),
                "OpenTermius native SFTP test fixture",
            );
            s.local_path = folder;
        });
        ui.invoke_open_sftp();
    }
    let timer = slint::Timer::default();
    if smoke || benchmark {
        let weak = ui.as_weak();
        let output = args
            .iter()
            .find_map(|a| a.strip_prefix("--output="))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("test-output"));
        if smoke_port.is_some() && smoke {
            let started = std::time::Instant::now();
            let mut stage = 0;
            let mut paint_at = std::time::Instant::now();
            let mut credentials = false;
            let mut fingerprint = false;
            timer.start(slint::TimerMode::Repeated,Duration::from_millis(50),move||{
                let result=(||->anyhow::Result<bool>{
                    let Some(ui)=weak.upgrade()else{return Ok(true)};
                    anyhow::ensure!(started.elapsed()<Duration::from_secs(18),"原生界面连接自检超时：{}",ui.get_status_text());
                    if ui.get_prompt_visible(){if ui.get_prompt_is_key(){anyhow::ensure!(ui.get_prompt_message().contains("SHA256:"),"未显示主机指纹");fingerprint=true;ui.invoke_answer_prompt(true,"".into());}else{credentials=true;ui.invoke_answer_prompt(true,"test-password".into());}}
                    let (connected,echo)=with_state(|s|s.active.as_ref().and_then(|id|s.sessions.get(id)).map(|v|(v.connected,v.snapshot.lines.iter().any(|l|l.contains("中文native-ui-test")))).unwrap_or_default()).unwrap_or_default();
                    if stage==0&&connected{ui.invoke_focus_terminal();ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed{text:"中文native-ui-test\r".into()});stage=1;}
                    if stage==1&&echo{ui.invoke_list_files(".".into());stage=2;}
                    if stage==2&&ui.get_files().row_count()>0{paint_at=std::time::Instant::now();stage=3;}
                    if stage==3&&paint_at.elapsed()>Duration::from_millis(250){
                        capture(&ui,&output)?;
                        let host=ui.get_hosts().row_data(0).context("界面主机丢失")?;
                        let session=ui.get_active_session();ui.invoke_close_session(session);
                        anyhow::ensure!(ui.get_tabs().row_count()==0,"关闭会话后标签未清理");
                        let mut draft=ui.invoke_edit_host(host.id.clone());draft.secret="test-password".into();draft.remember=true;anyhow::ensure!(ui.invoke_save_host(draft),"系统加密凭据保存失败");
                        with_state(|s|->anyhow::Result<()>{let store=s.store.lock().unwrap();let p=&store.data.hosts[0];anyhow::ensure!(store::unprotect(p.encrypted_secret.as_deref().context("缺少加密凭据")?)?=="test-password","系统凭据未正确解密");anyhow::ensure!(!std::fs::read_to_string(&store.path)?.contains("test-password"),"凭据被明文保存");anyhow::ensure!(Store::open(store.path.clone())?.data.hosts.len()==1,"重开配置失败");Ok(())}).unwrap()?;
                        ui.invoke_search_hosts("absent-search".into());anyhow::ensure!(ui.get_hosts().row_count()==0,"主机搜索失败");ui.invoke_search_hosts("".into());
                        anyhow::ensure!(credentials&&fingerprint,"未覆盖凭据和主机密钥弹窗");
                        anyhow::ensure!((ui.get_cell_width()-7.6972656).abs()<0.05,"Terminal cell metric does not match Consolas 14px");
                        std::fs::write(output.join("startup.json"),serde_json::json!({"ok":true,"version":"0.2.0","cellWidth":ui.get_cell_width(),"checks":["native profile callback","password prompt","explicit SHA256 prompt","real SSH terminal IO","real SFTP listing","tab cleanup","Windows DPAPI","no plaintext credentials","profile persistence","host search","measured terminal cell metric"],"pid":std::process::id()}).to_string())?;
                        return Ok(true)
                    }
                    Ok(false)
                })();
                match result{Ok(true)=>{let _=slint::quit_event_loop();},Err(error)=>{let _=std::fs::create_dir_all(&output);let _=std::fs::write(output.join("error.txt"),error.to_string());let _=slint::quit_event_loop();},_=>{}}
            });
        } else {
            timer.start(slint::TimerMode::SingleShot,Duration::from_millis(800),move||{
                if let Some(ui)=weak.upgrade(){
                    let result=(||->anyhow::Result<()>{capture(&ui,&output)?;std::fs::write(output.join("startup.json"),serde_json::json!({"ok":true,"version":"0.2.0","backend":"winit-software","scene":scene,"cellWidth":ui.get_cell_width(),"pid":std::process::id()}).to_string())?;Ok(())})();
                    if let Err(error)=result{let _=std::fs::write(output.join("error.txt"),error.to_string());}
                }
                if smoke{let _=slint::quit_event_loop();}
            });
        }
    }
    ui.run()?;
    STATE.with(|slot| {
        if let Some(mut state) = slot.borrow_mut().take() {
            for forward in state.forwards.values() { forward.request_stop(); }
            for (_, mut forward) in std::mem::take(&mut state.forwards) {
                state.runtime.block_on(forward.stop());
            }
            for (id, mut view) in std::mem::take(&mut state.sessions) {
                pages::logs::closed(&id);
                if let Some(transport) = view.transport.take() {
                    let _ = transport.commands.try_send(Command::Disconnect);
                    transport.task.abort();
                }
            }
            state.runtime.shutdown_timeout(Duration::from_secs(1));
        }
    });
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        rfd::MessageDialog::new()
            .set_title("OpenTermius 启动失败")
            .set_description(e.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1)
    }
}

#[cfg(test)]
mod terminal_ux_tests {
    use super::*;

    pub(super) fn state() -> State {
        let (events, _) = mpsc::channel(32);
        State {
            ui: slint::Weak::default(),
            store: Arc::new(Mutex::new(Store { path: PathBuf::new(), data: store::Data::default() })),
            runtime: tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap(),
            settings: Settings::default(),
            events, sessions: BTreeMap::new(), order: Vec::new(), active: None,
            queue: VecDeque::new(), prompt: None, cols: 80, rows: 24,
            file_path: String::new(), search: String::new(), group_filter: String::new(),
            local_path: PathBuf::new(), local_request: 0, pending_sftp: None,
            remote_entries: Vec::new(), local_entries: Vec::new(),
            transfers: BTreeMap::new(), next_transfer: 0,
            render_model: Rc::new(VecModel::from(Vec::new())),
            selecting: false, mouse_button: None,
            forwards: BTreeMap::new(), forward_errors: BTreeMap::new(),
            forwarding_timer: slint::Timer::default(),
        }
    }

    fn profile() -> Profile {
        Profile {
            id: "profile".into(), name: "Test SSH".into(), host: "localhost".into(), port: 2222,
            username: "tester".into(), auth_type: "password".into(),
            private_key_path: String::new(), group: "Development".into(), encrypted_secret: None,
        }
    }

    pub(super) fn session(state: &mut State) -> mpsc::Receiver<Command> {
        let (commands, rx) = mpsc::channel(32);
        let task = state.runtime.spawn(async {});
        state.sessions.insert("old".into(), SessionView {
            local: false, profile: profile(),
            transport: Some(ssh::Session { id: "old".into(), commands, task }),
            snapshot: Terminal::new(24, 80).snapshot(), status: "已连接".into(),
            connected: true, ended: false, selection: None,
        });
        state.order.push("old".into());
        state.active = Some("old".into());
        rx
    }

    fn input(rx: &mut mpsc::Receiver<Command>) -> Vec<u8> {
        match rx.try_recv().unwrap() {
            Command::Input(bytes) => bytes,
            other => panic!("expected input, got {other:?}"),
        }
    }

    #[test]
    fn queued_uploads_apply_overwrite_preference_to_files_and_folders() {
        let mut s = state();
        let mut rx = session(&mut s);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("upload.txt");
        std::fs::write(&file, b"settings upload").unwrap();
        for confirm in [true, false] {
            s.settings.confirm_overwrite = confirm;
            s.upload_path(file.clone());
            assert!(matches!(rx.try_recv().unwrap(), Command::UploadFile { overwrite, .. } if overwrite == !confirm));
            s.upload_path(temp.path().to_path_buf());
            assert!(matches!(rx.try_recv().unwrap(), Command::UploadTree { overwrite, .. } if overwrite == !confirm));
        }
        assert_eq!(s.transfers.len(), 4);
    }

    #[test]
    fn slint_keys_and_committed_text_reach_transport() {
        let mut s = state();
        let mut rx = session(&mut s);
        for (key, expected) in [
            (slint::platform::Key::F12, b"\x1b[24~".as_slice()),
            (slint::platform::Key::Backspace, b"\x7f".as_slice()),
            (slint::platform::Key::Backtab, b"\x1b[Z".as_slice()),
        ] {
            s.key(SharedString::from(key).as_str(), false, false, false);
            assert_eq!(input(&mut rx), expected);
        }
        s.key("中文输入", false, false, false);
        assert_eq!(input(&mut rx), "中文输入".as_bytes());
        s.ime("你好，世界");
        assert_eq!(input(&mut rx), "你好，世界".as_bytes());
        s.key(" ", true, false, false);
        assert_eq!(input(&mut rx), [0]);
        s.sessions.get_mut("old").unwrap().snapshot.application_cursor = true;
        s.key(SharedString::from(slint::platform::Key::UpArrow).as_str(), false, false, false);
        assert_eq!(input(&mut rx), b"\x1bOA");
        s.key(SharedString::from(slint::platform::Key::PageUp).as_str(), true, false, true);
        assert_eq!(input(&mut rx), b"\x1b[5;6~");
    }

    #[test]
    fn physical_keypad_mapping_and_application_dispatch() {
        use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
        let mut s = state();
        let mut rx = session(&mut s);
        for (code, character, expected) in [
            (KeyCode::Numpad0, '0', "\x1bOp"), (KeyCode::Numpad1, '1', "\x1bOq"),
            (KeyCode::Numpad2, '2', "\x1bOr"), (KeyCode::Numpad3, '3', "\x1bOs"),
            (KeyCode::Numpad4, '4', "\x1bOt"), (KeyCode::Numpad5, '5', "\x1bOu"),
            (KeyCode::Numpad6, '6', "\x1bOv"), (KeyCode::Numpad7, '7', "\x1bOw"),
            (KeyCode::Numpad8, '8', "\x1bOx"), (KeyCode::Numpad9, '9', "\x1bOy"),
            (KeyCode::NumpadDecimal, '.', "\x1bOn"), (KeyCode::NumpadComma, ',', "\x1bOl"),
            (KeyCode::NumpadAdd, '+', "\x1bOk"), (KeyCode::NumpadSubtract, '-', "\x1bOm"),
            (KeyCode::NumpadMultiply, '*', "\x1bOj"), (KeyCode::NumpadDivide, '/', "\x1bOo"),
            (KeyCode::NumpadEqual, '=', "\x1bOX"), (KeyCode::NumpadEnter, '\r', "\x1bOM"),
        ] {
            assert_eq!(keypad_key(PhysicalKey::Code(code)), Some(character));
            s.sessions.get_mut("old").unwrap().snapshot.application_keypad = false;
            assert!(!s.keypad(character, terminal::KeyMods::default()));
            s.sessions.get_mut("old").unwrap().snapshot.application_keypad = true;
            assert!(s.keypad(character, terminal::KeyMods::default()));
            assert_eq!(input(&mut rx), expected.as_bytes());
            // Main keyboard digits and Enter retain their normal encoding.
            let text = if character == '\r' { "\n".to_string() } else { character.to_string() };
            s.key(&text, false, false, false);
            assert_eq!(input(&mut rx), character.to_string().as_bytes());
        }
        assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Enter)), None);
        assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Digit1)), None);
    }

    #[test]
    fn wheel_and_pointer_follow_remote_mouse_modes() {
        let mut s = state();
        let mut rx = session(&mut s);
        s.wheel(0, 0, 3, false, false, false);
        assert!(matches!(rx.try_recv().unwrap(), Command::Scroll(3)));
        s.pointer(0, 1, 2, 3, false, false, false);
        assert!(!s.selecting); // Right-click never begins a text selection.
        s.pointer(0, 0, 2, 3, false, false, false);
        s.pointer(2, -1, 2, 8, false, false, false);
        s.pointer(1, 0, 2, 8, false, false, false);
        assert_eq!(s.sessions["old"].selection, Some(((2, 3), (2, 8))));
        let mut t = Terminal::new(24, 80);
        t.feed(b"\x1b[?1002h\x1b[?1006h");
        s.sessions.get_mut("old").unwrap().snapshot = t.snapshot();
        s.pointer(0, 0, 2, 3, true, false, false);
        assert_eq!(input(&mut rx), b"\x1b[<16;4;3M");
        s.pointer(2, -1, 3, 4, true, false, false);
        assert_eq!(input(&mut rx), b"\x1b[<48;5;4M");
        s.pointer(1, 0, 3, 4, true, false, false);
        assert_eq!(input(&mut rx), b"\x1b[<16;5;4m");
        s.wheel(3, 4, -2, false, true, false);
        assert_eq!(input(&mut rx), b"\x1b[<73;5;4M\x1b[<73;5;4M");
        s.wheel(3, 4, 2, false, false, true);
        assert!(matches!(rx.try_recv().unwrap(), Command::Scroll(2)));
        s.pointer(2, -1, 0, 0, false, false, false);
        assert!(rx.try_recv().is_err()); // Button-motion mode needs a held button.
    }

    #[test]
    fn reconnect_reuses_profile_without_reusing_transport_id() {
        let mut s = state();
        let _rx = session(&mut s);
        s.reconnect("old");
        assert_eq!(s.active.as_deref(), Some("old")); // Connected/connecting tabs are protected.
        s.event(Event::Closed { id: "old".into() });
        assert!(s.sessions["old"].ended);
        s.key(SharedString::from(slint::platform::Key::Return).as_str(), false, false, false);
        let new_id = s.active.clone().unwrap();
        assert_ne!(new_id, "old");
        assert_eq!(s.sessions.len(), 1);
        assert_eq!(s.order, [new_id.clone()]);
        let view = &s.sessions[&new_id];
        assert_eq!(view.profile.host, "localhost");
        assert_eq!(view.profile.port, 2222);
        assert_eq!(view.profile.username, "tester");
        assert!(!view.ended);
        assert!(matches!(s.prompt, Some(Prompt::Credential { .. })));
        s.event(Event::Closed { id: "old".into() });
        assert!(!s.sessions[&new_id].ended);
        s.answer(false, String::new());
        assert!(s.sessions[&new_id].ended);
    }

    #[test]
    fn duplicate_session_keeps_original_and_uses_its_profile() {
        let mut s = state();
        let _rx = session(&mut s);
        s.duplicate("old");
        assert_eq!(s.sessions.len(), 2);
        let id = s.active.clone().unwrap();
        assert_ne!(id, "old");
        assert_eq!(s.sessions[&id].profile.id, s.sessions["old"].profile.id);
        assert!(s.sessions["old"].connected);
        s.close(&id);
        assert_eq!(s.order, ["old"]);
        assert_eq!(s.active.as_deref(), Some("old"));
        assert!(s.prompt.is_none());
    }
}
