#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use anyhow::Context;
use opentermius::i18n;
use opentermius::{
    settings::{BellMode, Settings},
    hosts,
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
use opentermius_ui::*;
mod pages;
mod rendering;

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
    tag_filter: String,
    local_path: PathBuf,
    local_request: u64,
    pending_sftp: Option<String>,
    /// Password/passphrase typed at the connect prompt, saved to the host once it connects.
    typed_secrets: BTreeMap<String, zeroize::Zeroizing<String>>,
    /// Latest usage reported by each connected session.
    server_stats: BTreeMap<String, opentermius::server_stats::Stats>,
    /// Sessions whose tab currently shows its SFTP view instead of the terminal.
    sftp_views: std::collections::BTreeSet<String>,
    remote_entries: Vec<ssh::RemoteFile>,
    local_entries: Vec<ssh::LocalFile>,
    transfers: BTreeMap<u64, (String, TransferRow)>,
    next_transfer: u64,
    render_model: Rc<VecModel<TerminalRun>>,
    selecting: bool,
    mouse_button: Option<terminal::MouseButton>,
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
fn drag_window(ui: &AppWindow) {
    // winit's drag marks the window as dragging and, on Windows, posts the button release
    // the native move loop swallows; without it Slint keeps the title bar's pointer grab.
    use slint::winit_030::WinitWindowAccessor;
    ui.window().with_winit_window(|window| { let _ = window.drag_window(); });
    #[cfg(windows)]
    release_grab_after_drag(ui.as_weak());
}

// Windows does not always end the move loop with WM_EXITSIZEMOVE, and then nobody delivers
// the release. Once the button is physically up, release the grab ourselves; a release
// with nothing grabbed is ignored by Slint.
#[cfg(windows)]
fn release_grab_after_drag(ui: slint::Weak<AppWindow>) {
    use windows_sys::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};
    thread_local! { static TIMER: slint::Timer = slint::Timer::default(); }
    TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
            let held = unsafe {
                let button = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
                GetAsyncKeyState(button as i32) as u16 & 0x8000 != 0
            };
            if held {
                return;
            }
            if let Some(ui) = ui.upgrade() {
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased {
                    position: slint::LogicalPosition::new(-1.0, -1.0),
                    button: slint::platform::PointerEventButton::Left,
                });
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerExited);
            }
            TIMER.with(|timer| timer.stop());
        });
    });
}

// Terminal copy/paste goes through these so unit tests never touch the user's clipboard.
#[cfg(not(test))]
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
#[cfg(not(test))]
fn set_clipboard_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)).map_err(|e| e.to_string())
}
#[cfg(test)]
thread_local! { static TEST_CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) }; }
#[cfg(test)]
fn clipboard_text() -> Option<String> {
    TEST_CLIPBOARD.with(|clipboard| clipboard.borrow().clone())
}
#[cfg(test)]
fn set_clipboard_text(text: String) -> Result<(), String> {
    TEST_CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = Some(text));
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum ClipboardShortcut {
    Copy,
    Paste,
}

/// Windows Terminal conventions: Ctrl+C copies only while text is selected (otherwise it
/// stays an interrupt), Ctrl+V / Shift+Insert paste, Ctrl+Insert / Ctrl+Shift+C copy.
fn clipboard_shortcut(text: &str, ctrl: bool, alt: bool, shift: bool, selected: bool) -> Option<ClipboardShortcut> {
    if alt {
        return None;
    }
    let is = |letter: &str, control: &str| text.eq_ignore_ascii_case(letter) || text == control;
    let insert = matches!(terminal::Key::from_text(text), Some(terminal::Key::Named(terminal::NamedKey::Insert)));
    if ctrl && is("c", "\u{3}") && (shift || selected) || ctrl && !shift && insert {
        Some(ClipboardShortcut::Copy)
    } else if ctrl && is("v", "\u{16}") || shift && !ctrl && insert {
        Some(ClipboardShortcut::Paste)
    } else {
        None
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
            ui.set_status_text(i18n::translate(message.as_ref()).into());
            ui.set_status_error(error)
        }
    }
    fn hosts(&self) {
        if let (Some(ui), Ok(store)) = (self.ui.upgrade(), self.store.lock()) {
            ui.set_host_view_mode(store.data.hosts_view.view_mode().as_str().into());
            ui.set_host_sort_mode(store.data.hosts_view.sort_mode().as_str().into());
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
            ui.set_host_tags(model(
                hosts::tag_counts(&store.data.hosts)
                    .into_iter()
                    .map(|(name, count)| TagRow {
                        name: name.into(),
                        count,
                    })
                    .collect(),
            ));
            // The highlighted group pill always reflects the filter actually applied; a group
            // that no longer exists (renamed or emptied) falls back to "All".
            let ungrouped = store.data.hosts.iter().any(|p| p.group.is_empty());
            let filter_exists = self.group_filter.is_empty()
                || (self.group_filter == hosts::NO_GROUP && ungrouped)
                || store.data.hosts.iter().any(|p| p.group == self.group_filter);
            let group_filter = if filter_exists { self.group_filter.clone() } else { String::new() };
            ui.set_group_filter(group_filter.clone().into());
            let rows: Vec<HostRow> = hosts::visible_hosts(
                &store.data.hosts,
                &self.search,
                &group_filter,
                &self.tag_filter,
                store.data.hosts_view.sort_mode(),
            )
            .into_iter()
            .map(|p| {
                // Live usage comes from any connected terminal of this host.
                let live = self.order.iter().find_map(|id| {
                    self.sessions.get(id).filter(|s| s.connected && !s.local && s.profile.id == p.id).map(|_| id)
                });
                let stats = live.and_then(|id| self.server_stats.get(id)).copied().unwrap_or_default();
                let value = |v: Option<u8>| v.map_or(-1, i32::from);
                HostRow {
                online: live.is_some(),
                cpu: value(stats.cpu),
                memory: value(stats.memory),
                disk: value(stats.disk),
                id: p.id.clone().into(),
                name: p.name.clone().into(),
                address: format!("{}@{}:{}", p.username, p.host, p.port).into(),
                group: p.group.clone().into(),
                auth: if p.auth_type == "key" {
                    crate::i18n::t("SSH private key").into()
                } else {
                    crate::i18n::t("Password").into()
                },
                has_secret: p.encrypted_secret.is_some(),
                tags: p
                    .tags
                    .iter()
                    .map(|tag| format!("#{tag}"))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .into(),
            }})
            .collect();
            ui.set_ungrouped_count(store.data.hosts.iter().filter(|p| p.group.is_empty()).count() as i32);
            let selected = ui.get_selected_host();
            if !selected.is_empty() && !rows.iter().any(|row| row.id == selected) {
                ui.set_selected_host("".into());
                ui.set_selected_host_name("".into());
            } else if let Some(row) = rows.iter().find(|row| row.id == selected) {
                ui.set_selected_host_name(row.name.clone());
            }
            ui.set_hosts(model(rows));
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
                            title: if s.local { i18n::t("Local Terminal").into() } else { s.profile.name.clone().into() },
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
                ui.set_active_host(if view.local { i18n::t("Local Terminal").into() } else { view.profile.name.clone().into() });
                let scheme = self.settings.color_scheme.resolve(ui.global::<Theme>().get_dark());
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
                self.status(crate::i18n::t("Port must be between 1 and 65535"), true);
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
            key_id: d.key_id.to_string(),
            encrypted_secret: None,
            tags: hosts::parse_tags(&d.tags),
            last_connected: None,
            created_at: None,
        };
        let mut secret = d.secret.to_string();
        // A password typed in the editor is saved (encrypted) so connecting does not ask again.
        let remember = d.remember || !secret.is_empty();
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut s| s.save(p, Some(&secret), remember));
        secret.zeroize();
        match result {
            Ok(()) => {
                self.hosts();
                self.status(crate::i18n::t("Host saved"), false);
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
            let tags = hosts::tags_string(&p);
            HostDraft {
                id: p.id.into(),
                name: p.name.into(),
                host: p.host.into(),
                port: p.port.to_string().into(),
                username: p.username.into(),
                group: p.group.into(),
                auth_index: i32::from(p.auth_type == "key"),
                key_path: p.private_key_path.into(),
                key_id: p.key_id.into(),
                secret: SharedString::default(),
                remember: true,
                tags: tags.into(),
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
    fn set_host_view(&mut self, value: &str) {
        let mode = hosts::ViewMode::parse(value);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_host_view_mode(mode.as_str().into());
        }
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut store| {
                let previous = store.data.hosts_view.clone();
                store.data.hosts_view.set_mode(mode);
                if let Err(error) = store.persist() {
                    store.data.hosts_view = previous;
                    return Err(error);
                }
                Ok(())
            });
        if let Err(error) = result {
            if let (Ok(store), Some(ui)) = (self.store.lock(), self.ui.upgrade()) {
                ui.set_host_view_mode(store.data.hosts_view.view_mode().as_str().into());
            }
            self.status(opentermius::tf!("Cannot save view settings: {error}", "error" => format!("{}", error)), true);
        }
    }
    fn set_host_sort(&mut self, value: &str) {
        let sort = hosts::SortMode::parse(value);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_host_sort_mode(sort.as_str().into());
        }
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut store| {
                let previous = store.data.hosts_view.clone();
                store.data.hosts_view.set_sort(sort);
                if let Err(error) = store.persist() {
                    store.data.hosts_view = previous;
                    return Err(error);
                }
                Ok(())
            });
        self.hosts();
        if let Err(error) = result {
            if let (Ok(store), Some(ui)) = (self.store.lock(), self.ui.upgrade()) {
                ui.set_host_sort_mode(store.data.hosts_view.sort_mode().as_str().into());
            }
            self.status(opentermius::tf!("Cannot save sort settings: {error}", "error" => format!("{}", error)), true);
        }
    }
    fn set_tag_filter(&mut self, value: &str) {
        self.tag_filter = value.trim().to_string();
        self.hosts();
    }
    fn move_selection(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let current = self
            .ui
            .upgrade()
            .map(|ui| ui.get_selected_host().to_string())
            .unwrap_or_default();
        let target = self.store.lock().ok().and_then(|store| {
            let visible = hosts::visible_hosts(
                &store.data.hosts,
                &self.search,
                &self.group_filter,
                &self.tag_filter,
                store.data.hosts_view.sort_mode(),
            );
            let ids: Vec<String> = visible.iter().map(|p| p.id.clone()).collect();
            let next = hosts::next_selection(&ids, &current, delta)?;
            visible
                .into_iter()
                .find(|p| p.id == next)
                .map(|p| (p.id.clone(), p.name.clone()))
        });
        if let (Some((id, name)), Some(ui)) = (target, self.ui.upgrade()) {
            let index = ui.get_hosts().iter().position(|host| host.id == id).unwrap_or(0);
            ui.set_selected_host(id.into());
            ui.set_selected_host_name(name.into());
            ui.set_host_selection_index(index as i32);
        }
    }
    fn duplicate_host(&mut self, id: &str) {
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut store| hosts::duplicate_profile(&mut store, id));
        match result {
            Ok(_) => {
                self.hosts();
                self.status(crate::i18n::t("Host duplicated"), false);
            }
            Err(error) => self.status(error.to_string(), true),
        }
    }
    fn copy_address(&self, id: &str) {
        let address = self.store.lock().ok().and_then(|store| {
            store
                .data
                .hosts
                .iter()
                .find(|p| p.id == id)
                .map(hosts::copy_address)
        });
        match address {
            Some(address) => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(address))
                {
                    Ok(()) => self.status(crate::i18n::t("Address copied to clipboard"), false),
                    Err(error) => self.status(error.to_string(), true),
                }
            }
            None => self.status(crate::i18n::t("Host not found"), true),
        }
    }
    fn open_host_sftp(&mut self, id: &str) {
        self.search.clear();
        self.tag_filter.clear();
        self.hosts();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_search("".into());
            ui.set_host_tag_filter("".into());
            ui.set_sftp_page(true);
        }
        self.list_local(String::new());
        self.select_sftp(id);
    }
    fn rename_group(&mut self, old: &str, new: &str) {
        let new_name = new.trim().to_string();
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut store| hosts::rename_group(&mut store, old, &new_name));
        match result {
            Ok(count) => {
                if let Some(ui) = self.ui.upgrade() {
                    let mut draft = ui.get_draft();
                    if draft.group == old {
                        draft.group = new_name.clone().into();
                        ui.set_draft(draft);
                    }
                }
                if self.group_filter == old {
                    self.group_filter = new_name.clone();
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_search(new_name.clone().into());
                    }
                }
                self.hosts();
                self.status(opentermius::tf!("Group renamed ({count} hosts)", "count" => format!("{}", count)), false);
            }
            Err(error) => self.status(error.to_string(), true),
        }
    }
    fn connect(&mut self, host_id: &str) {
        let profile = self
            .store
            .lock()
            .ok()
            .and_then(|s| s.data.hosts.iter().find(|p| p.id == host_id).cloned());
        let Some(profile) = profile else {
            self.status(crate::i18n::t("Host not found"), true);
            return;
        };
        self.connect_profile(profile);
    }
    fn connect_profile(&mut self, profile: Profile) {
        if self.sessions.len() >= 32 {
            self.status(
                crate::i18n::t("Close an unused session before opening more than 32 tabs"),
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
                status: crate::i18n::t("Waiting for authentication").into(),
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
        if profile.auth_type == "key" && !profile.key_id.is_empty() {
            // The keychain material is protected by DPAPI, so no passphrase
            // prompt is needed on connect.
            self.start(&id, String::new());
            return;
        }
        let encrypted = profile.encrypted_secret;
        if let Some(value) = encrypted {
            match store::unprotect(&value) {
                Ok(secret) => self.start(&id, secret),
                Err(e) => {
                    self.status(opentermius::tf!("Cannot read saved credentials; enter them again: {e}", "e" => format!("{}", e)), true);
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
        let Some(profile) = self.sessions.get(id).map(|view| view.profile.clone()) else {
            return;
        };
        pages::logs::ssh_started(id, &profile);
        let key_material = if profile.auth_type == "key" && !profile.key_id.is_empty() {
            match self
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                .and_then(|store| store.key_material(&profile.key_id))
            {
                Ok(material) => Some(material),
                Err(error) => {
                    pages::logs::failed(id, &format!("{error:#}"));
                    if let Some(view) = self.sessions.get_mut(id) {
                        view.status = format!("{error:#}");
                    }
                    self.status(format!("{error:#}"), true);
                    self.tabs();
                    return;
                }
            }
        } else {
            None
        };
        let known = self.store.lock().ok().and_then(|s| {
            s.data
                .known_hosts
                .get(&store::key(&profile.host, profile.port))
                .cloned()
        });
        let runtime = self.runtime.handle().clone();
        let events = self.events.clone();
        if let Some(view) = self.sessions.get_mut(id) {
            view.status = crate::i18n::t("Connecting").into();
            view.transport = Some(ssh::spawn_with_key_material_and_options(
                &runtime,
                id.to_string(),
                profile,
                secret,
                key_material,
                known,
                events,
                ssh::ConnectOptions {
                    keepalive_seconds: self.settings.keepalive_seconds,
                    timeout_seconds: self.settings.connection_timeout_seconds,
                    scrollback_lines: self.settings.scrollback_lines,
                    initial_size: (self.cols, self.rows),
                },
            ));
        }
        self.tabs();
    }
    fn open_terminal(&mut self) {
        if self.sessions.len() >= 32 || self.sessions.values().filter(|s| s.local).count() >= 8 {
            self.status(
                crate::i18n::t("Close an unused terminal before opening another local session"),
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
            name: crate::i18n::t("Local Terminal").into(),
            host: "localhost".into(),
            port: 22,
            username: "local".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            group: String::new(),
            encrypted_secret: None,
            tags: Vec::new(),
            last_connected: None,
            created_at: None,
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
                status: crate::i18n::t("Starting").into(),
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
    /// Bring the active session's PTY to the last visible terminal size (a background tab
    /// keeps the size it had while the window was resized).
    fn fit_active(&self) {
        let stale = self
            .active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|view| view.transport.is_some() && (view.snapshot.rows, view.snapshot.cols) != (self.rows, self.cols));
        if stale {
            self.send(Command::Resize { cols: self.cols, rows: self.rows });
        }
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
                self.status(opentermius::tf!("Session is temporarily unavailable: {e}", "e" => format!("{}", e)), true)
            }
        }
    }
    fn close(&mut self, id: &str) {
        pages::logs::closed(id);
        self.typed_secrets.remove(id);
        self.server_stats.remove(id);
        self.sftp_views.remove(id);
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
                // Right click copies a selection, otherwise pastes (Windows console convention).
                0 if button == 1 => {
                    if self.has_selection() {
                        self.copy();
                        self.clear_selection();
                    } else {
                        self.paste();
                    }
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
                        ui.set_prompt_title(crate::i18n::t("SSH authentication").into());
                        ui.set_prompt_message(crate::i18n::t("Username").into());
                        ui.set_prompt_password(false);
                    }
                    Prompt::Credential { id } => {
                        let p = &self.sessions[id].profile;
                        let key = p.auth_type == "key";
                        ui.set_prompt_title(opentermius::tf!("Connect to {}", "0" => format!("{}", p.name)).into());
                        ui.set_prompt_message(if key {
                            crate::i18n::t("Private key passphrase (leave empty for an unencrypted key)").into()
                        } else {
                            opentermius::tf!("SSH password for {}@{}:{}", "0" => format!("{}", p.username), "1" => format!("{}", p.host), "2" => format!("{}", p.port)).into()
                        });
                    }
                    Prompt::Host {
                        host,
                        port,
                        fingerprint,
                        ..
                    } => {
                        ui.set_prompt_title(crate::i18n::t("First connection · Verify host key").into());
                        ui.set_prompt_message(
                            opentermius::tf!("{host}:{port}\n\n{fingerprint}\n\nVerify this fingerprint through a trusted channel before trusting it.", "host" => format!("{}", host), "port" => format!("{}", port), "fingerprint" => format!("{}", fingerprint))
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
                            opentermius::tf!("Server authentication {}/{}", "0" => format!("{}", index + 1), "1" => format!("{}", prompts.len())).into(),
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
                    view.status = crate::i18n::t("Cancelled").into();
                    view.ended = true;
                    self.tabs();
                    self.display();
                }
            }
            Some(Prompt::Credential { id }) => {
                if accepted {
                    if !answer.is_empty() {
                        self.typed_secrets.insert(id.clone(), zeroize::Zeroizing::new(answer.clone()));
                    }
                    self.start(&id, std::mem::take(&mut answer))
                } else if let Some(s) = self.sessions.get_mut(&id) {
                    s.status = crate::i18n::t("Cancelled").into();
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
                        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
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
            | Event::TransferFinished { id, .. } | Event::Stats { id, .. } => id,
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
                        crate::i18n::t("Connected").into()
                    } else {
                        message.clone()
                    };
                }
                if connected {
                    self.selecting = false;
                    self.mouse_button = None;
                    if let Some(ui) = self.ui.upgrade().filter(|ui| ui.get_terminal_shown()) {
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
                let mut connect_stamp_error = None;
                if connected {
                    pages::logs::connected(&id);
                    let profile_id = self
                        .sessions
                        .get(&id)
                        .filter(|session| !session.local)
                        .map(|session| session.profile.id.clone());
                    if let Some(profile_id) = &profile_id {
                        self.remember_typed_credentials(&id, profile_id);
                    }
                    if let Some(profile_id) = profile_id {
                        let now = hosts::now_rfc3339();
                        let result = self
                            .store
                            .lock()
                            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                            .and_then(|mut store| {
                                hosts::mark_connected(&mut store, &profile_id, &now)
                            });
                        match result {
                            Ok(true) => self.hosts(),
                            Ok(false) => {}
                            Err(error) => {
                                connect_stamp_error =
                                    Some(opentermius::tf!("Connected, but cannot save last connection time: {error}", "error" => format!("{}", error)))
                            }
                        }
                    }
                } else if error {
                    // A rejected password must not be saved.
                    self.typed_secrets.remove(&id);
                    pages::logs::failed(&id, &message);
                }
                self.status(
                    if connected {
                        crate::i18n::t("Connected").to_string()
                    } else {
                        message
                    },
                    error,
                );
                if let Some(message) = connect_stamp_error {
                    self.status(message, true);
                }
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
                local_root,
                id,
                directory,
                local,
                transfer,
                folder,
            } => {
                let confirmed = rfd::MessageDialog::new()
                    .set_title(crate::i18n::t("Overwrite remote file"))
                    .set_description(if folder {
                        crate::i18n::t("A folder with this name already exists. Merge and overwrite matching files?")
                    } else {
                        crate::i18n::t("A file with this name already exists. Overwrite it?")
                    })
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if confirmed {
                    if let Some(transport) = self.sessions.get(&id).and_then(|s| s.transport.as_ref()) {
                        let command = if transfer == 0 {
                            Command::Upload {
                                local_root,
                                directory,
                                local,
                                overwrite: true,
                            }
                        } else if folder {
                            Command::UploadTree {
                                local_root,
                                transfer,
                                directory,
                                local,
                                overwrite: true,
                            }
                        } else {
                            Command::UploadFile {
                                local_root,
                                transfer,
                                directory,
                                local,
                                overwrite: true,
                            }
                        };
                        if transport.commands.try_send(command).is_err() {
                            self.finish_transfer(transfer, crate::i18n::t("Transfer queue is busy").into(), true, false);
                        }
                    }
                } else {
                    self.finish_transfer(transfer, crate::i18n::t("Transfer cancelled").into(), false, true);
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_file_busy(false)
                    }
                }
            }
            Event::Stats { id, stats } => {
                // Rows are rebuilt below; a status change alone also refreshes them.
                self.server_stats.insert(id, stats);
                self.hosts();
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
                    .set_title(crate::i18n::t("Confirm deletion"))
                    .set_description(opentermius::tf!("Delete {name}?\n{files} files, {dirs} subfolders, {}. This cannot be undone.", "name" => format!("{}", name), "files" => format!("{}", files), "dirs" => format!("{}", dirs), "0" => format!("{}", format_size(bytes))))
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
                            self.status(crate::i18n::t("File task queue is busy"), true);
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
                    row.name = format!("{} · {name}", i18n::translate(&direction)).into();
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
                        row.detail = crate::i18n::t("Connection closed; transfer cancelled").into();
                    }
                }
                self.render_transfers();
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.connected = false;
                    s.ended = true;
                    if s.status == crate::i18n::t("Connected") || s.status == crate::i18n::t("Connecting") {
                        s.status = crate::i18n::t("Disconnected").into();
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
        match clipboard_shortcut(text, ctrl, alt, shift, self.has_selection()) {
            Some(ClipboardShortcut::Copy) => {
                self.copy();
                self.clear_selection();
                return;
            }
            Some(ClipboardShortcut::Paste) => {
                self.paste();
                return;
            }
            None => {}
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
            if let Err(e) = set_clipboard_text(text) {
                self.status(e, true)
            }
        }
    }
    fn has_selection(&self) -> bool {
        self.active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|view| view.selection.is_some_and(|(a, b)| a != b))
    }
    fn clear_selection(&mut self) {
        if let Some(view) = self.active.as_ref().and_then(|id| self.sessions.get_mut(id)) {
            view.selection = None;
        }
        self.display();
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
        if let Err(e) = set_clipboard_text(text) {
            self.status(e, true)
        }
    }
    fn paste(&self) {
        {
            if let Some(text) = clipboard_text().filter(|text| !text.is_empty()) {
                if text.len() > 1024 * 1024 {
                    self.status(crate::i18n::t("Pasted text must not exceed 1 MB"), true);
                    return;
                }
                let lines = opentermius::settings::paste_line_count(&text);
                if self.settings.confirm_multiline_paste && lines > 1 {
                    let confirmed = rfd::MessageDialog::new()
                        .set_title(crate::i18n::t("Paste multiple lines"))
                        .set_description(opentermius::tf!("The clipboard contains {lines} lines. Paste them into the terminal?", "lines" => format!("{}", lines)))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        == rfd::MessageDialogResult::Yes;
                    if !confirmed {
                        self.status(crate::i18n::t("Paste cancelled"), false);
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
            self.status(crate::i18n::t("Select a connected remote host first"), true);
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
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
                let path = ssh::canonical_local_root(&path)?;
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
                            ui.set_local_path(ssh::display_local_path(&s.local_path).into());
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
            self.status(crate::i18n::t("Select a connected remote host first"), true);
            return;
        }
        if self
            .transfers
            .values()
            .filter(|(_, row)| row.pending)
            .count()
            >= 8
        {
            self.status(crate::i18n::t("Transfer queue is full (up to 8 tasks). Wait or cancel a task"), true);
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
            detail: crate::i18n::t("Queued").into(),
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
            self.finish_transfer(transfer, crate::i18n::t("Transfer queue is busy").into(), true, false);
        }
        self.render_transfers();
    }
    fn upload_path(&mut self, local: PathBuf) {
        let local_root = match local.parent().map(ssh::canonical_local_root) {
            Some(Ok(root)) => root,
            Some(Err(error)) => {
                self.status(error.to_string(), true);
                return;
            }
            None => {
                self.status(crate::i18n::t("Invalid local file path"), true);
                return;
            }
        };
        let local = local_root.join(local.file_name().unwrap_or_default());
        let name = local
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let directory = self.file_path.clone();
        let folder = local.is_dir();
        let overwrite = !self.settings.confirm_overwrite;
        self.queue_transfer(&name, crate::i18n::t("Upload"), |transfer| {
            if folder {
                Command::UploadTree {
                    local_root,
                    transfer,
                    directory,
                    local,
                    overwrite,
                }
            } else {
                Command::UploadFile {
                    local_root,
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
            self.status(crate::i18n::t("Remote file name cannot be used in a local path"), true);
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
        let chosen_root = if folder { local.as_path() } else { local.parent().unwrap_or(&local) };
        let local_root = match ssh::canonical_local_root(chosen_root) {
            Ok(root) => root,
            Err(error) => {
                self.status(error.to_string(), true);
                return;
            }
        };
        let local = if folder {
            local_root.clone()
        } else {
            local_root.join(local.file_name().unwrap_or_default())
        };
        self.queue_transfer(name, crate::i18n::t("Download"), |transfer| {
            if folder {
                Command::DownloadTree {
                    local_root,
                    transfer,
                    remote,
                    local_dir: local,
                }
            } else {
                Command::DownloadFile {
                    local_root,
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
            self.status(crate::i18n::t("Invalid file name"), true);
            return;
        }
        if !local {
            if !self.remote_ready() {
                self.status(crate::i18n::t("Connect to a remote host first"), true);
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
                        .set_title(crate::i18n::t("Confirm deletion"))
                        .set_description(opentermius::tf!("Delete {}?\n{} files, {} subfolders, {}. This cannot be undone.", "0" => format!("{}", path.display()), "1" => format!("{}", summary.files), "2" => format!("{}", summary.dirs), "3" => format!("{}", format_size(summary.bytes))))
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
                                            Ok(()) => s.status(crate::i18n::t("Deleted"), false),
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
                    anyhow::ensure!(!destination.try_exists()?, "{}", crate::i18n::t("Destination name already exists"));
                    match operation.as_str() {
                        "mkdir" => std::fs::create_dir(destination)?,
                        "rename" => std::fs::rename(path, destination)?,
                        _ => anyhow::bail!("{}", crate::i18n::t("Unsupported file operation")),
                    }
                    Ok(())
                })();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    with_state(|s| {
                        match result {
                            Ok(()) => s.status(crate::i18n::t("File operation completed"), false),
                            Err(e) => s.status(e.to_string(), true),
                        };
                        s.list_local(String::new());
                    });
                });
            }
        });
    }
    /// After a successful login, keep what the user typed at the prompt (password and a
    /// username the saved host did not have) so the next connection does not ask again.
    fn remember_typed_credentials(&mut self, id: &str, profile_id: &str) {
        let secret = self.typed_secrets.remove(id);
        let username = self.sessions.get(id).map(|s| s.profile.username.clone()).unwrap_or_default();
        let result = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
            .and_then(|mut store| {
                let Some(mut profile) = store.data.hosts.iter().find(|p| p.id == profile_id).cloned() else { return Ok(false) };
                let new_username = profile.username.is_empty() && !username.is_empty();
                if secret.is_none() && !new_username {
                    return Ok(false);
                }
                if new_username {
                    profile.username = username;
                }
                store.save(profile, secret.as_deref().map(String::as_str), true).map(|()| true)
            });
        match result {
            Ok(true) => self.hosts(),
            Ok(false) => {}
            Err(error) => self.status(opentermius::tf!("Connected, but cannot save the credentials: {error}", "error" => format!("{error}")), true),
        }
    }
    fn show_sftp(&mut self, on: bool) {
        let local = self.active.as_ref().and_then(|id| self.sessions.get(id)).is_some_and(|s| s.local);
        let on = on && !local;
        if let Some(id) = self.active.clone() {
            if on {
                self.sftp_views.insert(id);
            } else {
                self.sftp_views.remove(&id);
            }
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_sftp_page(on);
        }
        if on {
            self.list_local(String::new());
            if self.remote_ready() {
                self.list_files(".".into());
            }
        }
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
            self.active = Some(id.clone());
            self.sftp_views.insert(id);
            self.reset_files();
            self.display();
            self.list_files(".".into());
            return;
        }
        self.connect(host_id);
        self.pending_sftp = self.active.clone();
        self.sftp_views.extend(self.active.clone());
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
            crate::i18n::t("Folder").into()
        } else {
            std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or(crate::i18n::t("File"))
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

fn startup_store(args: &[String]) -> anyhow::Result<Store> {
    let isolated_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--data-dir="))
        .map(|p| PathBuf::from(p).join("opentermius-native.json"));
    #[cfg(unix)]
    if let Some(path) = &isolated_path {
        store::configure_secret_directory(path.parent().context(crate::i18n::t("Invalid data directory"))?.to_owned())?;
    }
    match isolated_path {
        Some(path) => Store::open(path),
        None => store::open_default(),
    }
}

fn run(store: Store, renderer: rendering::Renderer, started: std::time::Instant, first_frame: Rc<std::cell::Cell<Option<f64>>>) -> anyhow::Result<()> {
    i18n::set_language(store.data.settings.language);
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().any(|a| a == "--smoke-test" || a == "--benchmark").then(|| {
        args.iter().find_map(|a| a.strip_prefix("--output=")).map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("test-output"))
    });
    let actual_renderer = Rc::new(std::cell::Cell::new(renderer));
    rendering::set_glass(renderer != rendering::Renderer::Software);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.slint_name().into())
        .with_winit_window_attributes_hook(rendering::window_attributes)
        .with_winit_custom_application_handler(rendering::WindowPolish::new(started, actual_renderer.clone(), output, first_frame.clone()))
        .select().context(rendering::PlatformFailure)?;
    let fonts = rendering::configure_fonts();
    let ui = AppWindow::new().context(rendering::PlatformFailure)?;
    // Slint's Windows GL probe may already have selected software internally.
    let renderer = if renderer != rendering::Renderer::Software && matches!(
        ui.window().set_rendering_notifier(|_, _| {}),
        Err(slint::SetRenderingNotifierError::Unsupported)
    ) { rendering::Renderer::Software } else { renderer };
    actual_renderer.set(renderer);
    // Slint may have fallen back to software rendering, which cannot draw translucency.
    rendering::set_glass(renderer != rendering::Renderer::Software);
    ui.global::<Theme>().set_translucent(rendering::glass());
    ui.global::<Theme>().set_ui_font(fonts.0.into());
    ui.global::<Theme>().set_mono_font(fonts.1.into());
    ui.set_terminal_font(fonts.1.into());
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
    anyhow::ensure!(!smoke
            || (scene == "empty" && smoke_port.is_none())
            || args.iter().any(|a| a.starts_with("--data-dir=")), "{}", crate::i18n::t("An isolated --data-dir is required when writing smoke test data"));
    let store = Arc::new(Mutex::new(store));
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
    let initial_local_path = ssh::canonical_local_root(&dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))?;
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
            tag_filter: String::new(),
            local_path: initial_local_path,
            local_request: 0,
            pending_sftp: None,
            sftp_views: Default::default(),
            typed_secrets: BTreeMap::new(),
            server_stats: BTreeMap::new(),
            remote_entries: Vec::new(),
            local_entries: Vec::new(),
            transfers: BTreeMap::new(),
            next_transfer: 0,
            render_model: Rc::new(VecModel::from(Vec::new())),
            selecting: false,
            mouse_button: None,
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
            drag_window(&ui);
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
    ui.on_set_host_view(|value| {
        with_state(|s| s.set_host_view(&value));
    });
    ui.on_set_host_sort(|value| {
        with_state(|s| s.set_host_sort(&value));
    });
    ui.on_set_host_tag_filter(|value| {
        with_state(|s| s.set_tag_filter(&value));
    });
    ui.on_move_host_selection(|delta| {
        with_state(|s| s.move_selection(delta));
    });
    ui.on_duplicate_host(|id| {
        with_state(|s| s.duplicate_host(&id));
    });
    ui.on_copy_host_address(|id| {
        with_state(|s| s.copy_address(&id));
    });
    ui.on_open_host_sftp(|id| {
        with_state(|s| s.open_host_sftp(&id));
    });
    ui.on_rename_group(|old, new| {
        with_state(|s| s.rename_group(&old, &new));
    });
    ui.on_suggest_host_groups(|query| {
        with_state(|s| {
            let groups = s.store.lock().map(|store| {
                hosts::group_suggestions(&store.data.hosts, &query)
                    .into_iter()
                    .map(|(name, count)| GroupRow { name: name.into(), count })
                    .collect()
            }).unwrap_or_default();
            model(groups)
        }).unwrap_or_default()
    });
    ui.on_edit_host(|id| with_state(|s| s.edit(&id)).unwrap_or_default());
    ui.on_delete_host(|id| {
        with_state(|s| {
            let result = s
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
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
            if let Some(ui) = s.ui.upgrade() {
                ui.set_sftp_page(s.active.as_ref().is_some_and(|id| s.sftp_views.contains(id)));
            }
            s.display();
            s.fit_active();
            if s.remote_ready()
                && s.ui
                    .upgrade()
                    .is_some_and(|ui| ui.get_sftp_page())
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
            .set_title(crate::i18n::t("Select SSH private key"))
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned().into())
            .unwrap_or_default()
    });
    ui.on_list_files(|path| {
        with_state(|s| s.list_files(path.to_string()));
    });
    ui.on_session_sftp(|on| {
        with_state(|s| {
            s.search.clear();
            s.hosts();
            s.show_sftp(on);
        });
    });
    ui.on_list_local(|path| {
        with_state(|s| s.list_local(path.to_string()));
    });
    ui.on_upload_local(|name| {
        with_state(|s| {
            if !safe_filename(&name) {
                s.status(crate::i18n::t("Invalid local file name"), true);
                return;
            }
            if !s.remote_ready() {
                s.status(crate::i18n::t("Select a remote host first"), true);
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
        if let Some(local) = rfd::FileDialog::new().set_title(crate::i18n::t("Upload file")).pick_file() {
            with_state(|s| {
                s.upload_path(local);
            });
        }
    });
    ui.on_download_file(|name| {
        if let Some(local) = rfd::FileDialog::new()
            .set_title(crate::i18n::t("Save downloaded file"))
            .set_file_name(name.as_str())
            .save_file()
        {
            with_state(|s| {
                s.download_entry(&name, Some(local));
            });
        }
    });
    ui.on_upload_folder(|| {
        if let Some(local) = rfd::FileDialog::new().set_title(crate::i18n::t("Upload folder")).pick_folder() {
            with_state(|s| match ssh::canonical_local_root(&local) {
                Ok(local) => s.upload_path(local),
                Err(error) => s.status(error.to_string(), true),
            });
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
                        row.detail = crate::i18n::t("Cancelling…").into();
                    } else {
                        s.status(crate::i18n::t("Cancellation request was not sent; try again"), true);
                    }
                }
            }
            s.render_transfers();
        });
    });
    ui.on_import_hosts(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(crate::i18n::t("Import hosts"))
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            with_state(|s| {
                let result = s
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                    .and_then(|mut store| store.import(&path));
                match result {
                    Ok(n) => {
                        s.hosts();
                        s.status(opentermius::tf!("Imported {n} hosts", "n" => format!("{}", n)), false)
                    }
                    Err(e) => s.status(e.to_string(), true),
                }
            });
        }
    });
    ui.on_import_xshell(|| {
        let default = opentermius::import_xshell::default_sessions_dir();
        let mut picker = rfd::FileDialog::new().set_title(crate::i18n::t("Import from Xshell — Select Sessions folder"));
        if let Some(path) = &default {
            picker = picker.set_directory(path);
        }
        let choice = default.as_ref().map(|path| {
            rfd::MessageDialog::new()
                .set_title(crate::i18n::t("Import from Xshell"))
                .set_description(opentermius::tf!("Import sessions from {}?\nChoose No to select another folder.", "0" => format!("{}", path.display())))
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
                    .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
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
                    .set_title(crate::i18n::t("Import from Xshell"))
                    .set_description(message)
                    .show();
            }
        }
    });
    ui.on_import_windterm(|| {
        use opentermius::import_windterm;
        let home = dirs::home_dir().unwrap_or_default();
        let found = import_windterm::detect(&home);
        let title = crate::i18n::t("Import from WindTerm");
        let files = if found.is_empty() {
            // Nothing in the usual places: let the user point at the WindTerm folder.
            match rfd::FileDialog::new().set_title(crate::i18n::t("Import from WindTerm — Select the WindTerm folder")).pick_folder() {
                Some(folder) => import_windterm::find_sessions(&folder),
                None => return,
            }
        } else {
            let list = found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            match rfd::MessageDialog::new()
                .set_title(title)
                .set_description(opentermius::tf!("Import WindTerm sessions from:\n{}\nChoose No to select another folder.", "0" => list))
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show()
            {
                rfd::MessageDialogResult::Yes => found,
                rfd::MessageDialogResult::No => match rfd::FileDialog::new().set_title(crate::i18n::t("Import from WindTerm — Select the WindTerm folder")).pick_folder() {
                    Some(folder) => import_windterm::find_sessions(&folder),
                    None => return,
                },
                _ => return,
            }
        };
        let scanned = if files.is_empty() {
            Err(anyhow::anyhow!("{}", crate::i18n::t("No WindTerm sessions were found in that folder")))
        } else {
            files.iter().try_fold(opentermius::import_xshell::ScanResult::default(), |mut all, file| {
                let mut one = import_windterm::scan_file(file, &home)?;
                all.hosts.append(&mut one.hosts);
                all.skipped.append(&mut one.skipped);
                Ok::<_, anyhow::Error>(all)
            })
        };
        let message = with_state(|s| {
            let result = scanned.and_then(|scanned| {
                s.store.lock()
                    .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                    .and_then(|mut store| opentermius::import_xshell::merge(&mut store, scanned))
            });
            match result {
                Ok(summary) => {
                    s.hosts();
                    let message = summary.message();
                    s.status(&message, false);
                    message
                }
                Err(error) => {
                    s.status(error.to_string(), true);
                    error.to_string()
                }
            }
        });
        if let Some(message) = message {
            rfd::MessageDialog::new().set_title(title).set_description(message).show();
        }
    });
    ui.on_export_hosts(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(crate::i18n::t("Export hosts (without credentials)"))
            .add_filter("JSON", &["json"])
            .set_file_name("opentermius-hosts.json")
            .save_file()
        {
            with_state(|s| {
                let result = s
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                    .and_then(|store| store.export(&path));
                s.status(
                    result
                        .map(|_| crate::i18n::t("Host configuration exported (without credentials)").to_string())
                        .unwrap_or_else(|e| e.to_string()),
                    false,
                )
            });
        }
    });
    pages::install(&ui);
    with_state(|s| {
        let view = s
            .store
            .lock()
            .map(|store| store.data.hosts_view.clone())
            .unwrap_or_default()
            .normalized();
        if let Some(ui) = s.ui.upgrade() {
            ui.set_host_view_mode(view.mode.into());
            ui.set_host_sort_mode(view.sort.into());
        }
        s.hosts();
        s.tabs();
        s.display()
    });
    if smoke && (scene == "hosts" || smoke_port.is_some()) {
        anyhow::ensure!(ui.invoke_save_host(HostDraft {
                id: "smoke-profile".into(),
                name: crate::i18n::t("Local test server").into(),
                host: "127.0.0.1".into(),
                port: smoke_port.unwrap_or(22).to_string().into(),
                username: "tester".into(),
                group: crate::i18n::t("Development").into(),
                ..HostDraft::default()
            }), "{}", crate::i18n::t("Failed to create host through the UI"));
        anyhow::ensure!(ui.get_hosts().row_count() == 1, "{}", crate::i18n::t("Host list was not updated"));
        if smoke_port.is_some() {
            let id = ui.get_hosts().row_data(0).unwrap().id;
            ui.invoke_connect_host(id)
        }
    }
    if smoke && scene == "host-cards" {
        // Synthetic hosts; three have fake connected sessions with usage figures.
        with_state(|s| {
            let samples = [
                ("jp", "JP VPS", "VPS", Some((5, 29, 49))),
                ("au", "AU VPS", "VPS", Some((0, 4, 2))),
                ("us", "US VPS", "VPS", Some((74, 61, 93))),
                ("de", "DE build box", "Lab", None),
                ("nas", "Home NAS", "", None),
            ];
            for (id, name, group, usage) in samples {
                let profile = Profile {
                    id: id.into(), name: name.into(), host: format!("{id}.example.invalid"), port: 22,
                    username: "dev".into(), auth_type: "password".into(), private_key_path: String::new(),
                    key_id: String::new(), group: group.into(), encrypted_secret: None, tags: Vec::new(),
                    last_connected: None, created_at: None,
                };
                let _ = s.store.lock().unwrap().save(profile.clone(), None, false);
                if let Some((cpu, memory, disk)) = usage {
                    let session = format!("cards-{id}");
                    s.sessions.insert(session.clone(), SessionView {
                        local: false, profile, transport: None, snapshot: Terminal::new(s.rows, s.cols).snapshot(),
                        status: crate::i18n::t("Connected").into(), connected: true, ended: false, selection: None,
                    });
                    s.order.push(session.clone());
                    s.server_stats.insert(session, opentermius::server_stats::Stats { cpu: Some(cpu), memory: Some(memory), disk: Some(disk) });
                }
            }
            s.tabs();
            s.hosts();
        });
        ui.invoke_open_section("hosts".into());
    }
    if smoke && scene == "editor" {
        ui.invoke_new_host();
    }
    if smoke && (scene == "section-terminal" || scene == "section-terminal-menu" || (scene == "section-sftp" && smoke_port.is_none())) {
        // Deterministic disconnected SSH fixture; no network or saved secrets. The SFTP
        // scene shows the same session switched to its file view.
        with_state(|s| {
            let mut terminal = Terminal::new(s.rows, s.cols);
            terminal.feed("\x1b[32mdev@workspace\x1b[0m:~$ echo 'Terminal ready · 中文输入'\r\nTerminal ready · 中文输入\r\n\r\n\x1b[33mSSH session ended.\x1b[0m\r\n".as_bytes());
            let id = "terminal-smoke".to_string();
            s.sessions.insert(id.clone(), SessionView {
                local: false,
                profile: Profile {
                    id: "smoke-profile".into(), name: crate::i18n::t("Development").into(), host: "localhost".into(),
                    port: 22, username: "dev".into(), auth_type: "password".into(),
                    private_key_path: String::new(), key_id: String::new(), group: String::new(), encrypted_secret: None,
                    tags: Vec::new(), last_connected: None, created_at: None,
                },
                transport: None, snapshot: terminal.snapshot(), status: crate::i18n::t("Disconnected").into(),
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
        ui.invoke_session_sftp(true);
        if scene == "section-sftp" && smoke_port.is_none() {
            // Disconnected visual fixture: exercise path input and partial progress without IO.
            ui.set_file_path("/srv/workspace/releases".into());
            ui.set_transfers(model(vec![TransferRow {
                id: "smoke-transfer".into(), name: "release.tar.gz".into(),
                detail: "42% · 4.2 MiB / 10 MiB".into(), percent: 42, pending: false,
            }]));
        }
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
                    anyhow::ensure!(started.elapsed()<Duration::from_secs(18), "{}", opentermius::tf!("Native UI connection self-test timed out: {}", "0" => format!("{}", ui.get_status_text())));
                    if ui.get_prompt_visible(){if ui.get_prompt_is_key(){anyhow::ensure!(ui.get_prompt_message().contains("SHA256:"), "{}", crate::i18n::t("Host fingerprint was not displayed"));fingerprint=true;ui.invoke_answer_prompt(true,"".into());}else{credentials=true;ui.invoke_answer_prompt(true,"test-password".into());}}
                    let (connected,echo)=with_state(|s|s.active.as_ref().and_then(|id|s.sessions.get(id)).map(|v|(v.connected,v.snapshot.lines.iter().any(|l|l.contains("中文native-ui-test")))).unwrap_or_default()).unwrap_or_default();
                    if stage==0&&connected{
                        with_state(|s| -> anyhow::Result<()> {
                            let store = s.store.lock().unwrap();
                            let host = &store.data.hosts[0];
                            let stamp = host.last_connected.as_deref().context(crate::i18n::t("Last connection time is missing"))?;
                            chrono::DateTime::parse_from_rfc3339(stamp)?;
                            let book = opentermius::logs::LogBook::open(store.path.parent().context(crate::i18n::t("Invalid configuration directory"))?)?;
                            anyhow::ensure!(book.entries().iter().any(|entry| entry.host_id == host.id && entry.status == opentermius::logs::CONNECTED), "{}", crate::i18n::t("Successful connection was not recorded in history"));
                            Ok(())
                        }).unwrap()?;
                        ui.invoke_focus_terminal();ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed{text:"中文native-ui-test\r".into()});stage=1;
                    }
                    if stage==1&&echo{ui.invoke_list_files(".".into());stage=2;}
                    if stage==2&&ui.get_files().row_count()>0{paint_at=std::time::Instant::now();stage=3;}
                    if stage==3&&paint_at.elapsed()>Duration::from_millis(250){
                        capture(&ui,&output)?;
                        let host=ui.get_hosts().row_data(0).context(crate::i18n::t("Host is missing from UI"))?;
                        let session=ui.get_active_session();ui.invoke_close_session(session);
                        anyhow::ensure!(ui.get_tabs().row_count()==0, "{}", crate::i18n::t("Tab was not removed after closing session"));
                        let mut draft=ui.invoke_edit_host(host.id.clone());draft.secret="test-password".into();draft.remember=true;anyhow::ensure!(ui.invoke_save_host(draft), "{}", crate::i18n::t("Failed to save system-encrypted credentials"));
                        with_state(|s|->anyhow::Result<()>{let store=s.store.lock().unwrap();let p=&store.data.hosts[0];anyhow::ensure!(store::unprotect(p.encrypted_secret.as_deref().context(crate::i18n::t("Encrypted credentials are missing"))?)?=="test-password", "{}", crate::i18n::t("System credentials did not decrypt correctly"));anyhow::ensure!(!std::fs::read_to_string(&store.path)?.contains("test-password"), "{}", crate::i18n::t("Credentials were stored in plaintext"));anyhow::ensure!(Store::open(store.path.clone())?.data.hosts.len()==1, "{}", crate::i18n::t("Failed to reopen configuration"));Ok(())}).unwrap()?;
                        ui.invoke_search_hosts("absent-search".into());anyhow::ensure!(ui.get_hosts().row_count()==0, "{}", crate::i18n::t("Host search failed"));ui.invoke_search_hosts("".into());
                        anyhow::ensure!(credentials&&fingerprint, "{}", crate::i18n::t("Credential and host key dialogs were not exercised"));
                        anyhow::ensure!(ui.get_cell_width().is_finite() && ui.get_cell_width() > 0.0 && ui.get_cell_height().is_finite() && ui.get_cell_height() > 0.0,"Terminal cell metric must be measured from the selected font");
                        std::fs::write(output.join("startup.json"),serde_json::json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"backend":renderer.backend(),"firstFrameMs":first_frame.get(),"terminalFont":ui.get_terminal_font().as_str(),"cellWidth":ui.get_cell_width(),"checks":["native profile callback","password prompt","explicit SHA256 prompt","real SSH terminal IO","real SFTP listing","tab cleanup",if cfg!(windows){"Windows DPAPI"}else{"AES-256-GCM"},"no plaintext credentials","profile persistence","host search","measured terminal cell metric"],"pid":rendering::launcher_pid(),"rendererPid":std::process::id()}).to_string())?;
                        return Ok(true)
                    }
                    Ok(false)
                })();
                match result{Ok(true)=>{let _=slint::quit_event_loop();},Err(error)=>{let _=std::fs::create_dir_all(&output);let _=std::fs::write(output.join("error.txt"),error.to_string());let _=slint::quit_event_loop();},_=>{}}
            });
        } else {
            timer.start(slint::TimerMode::SingleShot,Duration::from_millis(800),move||{
                if let Some(ui)=weak.upgrade(){
                    let result=(||->anyhow::Result<()>{capture(&ui,&output)?;std::fs::write(output.join("startup.json"),serde_json::json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"backend":renderer.backend(),"firstFrameMs":first_frame.get(),"terminalFont":ui.get_terminal_font().as_str(),"scene":scene,"cellWidth":ui.get_cell_width(),"pid":rendering::launcher_pid(),"rendererPid":std::process::id()}).to_string())?;Ok(())})();
                    if let Err(error)=result{let _=std::fs::write(output.join("error.txt"),error.to_string());}
                }
                if smoke{let _=slint::quit_event_loop();}
            });
        }
    }
    let platform_result = ui.run().context(rendering::PlatformFailure);
    STATE.with(|slot| {
        if let Some(mut state) = slot.borrow_mut().take() {
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
    platform_result
}
fn main() {
    let started = std::time::Instant::now();
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.get(1).is_some_and(|a| a == "--apply-update") {
        // Helper copy started by the updater: no window, no configuration access.
        if let Err(error) = opentermius::update::apply(&args[2..]) {
            rfd::MessageDialog::new()
                .set_title(crate::i18n::t("Update failed"))
                .set_description(format!("{error:#}"))
                .show();
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    let result = (|| -> anyhow::Result<i32> {
        let store = startup_store(&args)?;
        let requested = rendering::requested(&args, std::env::var("OPENTERMIUS_RENDERER").ok().as_deref(), store.data.settings.gpu_rendering || store.data.settings.glass_effect)?;
        let renderer = if requested.available() { requested } else {
            eprintln!("{}", opentermius::tf!("This build does not include the {} renderer; falling back to software rendering", "0" => format!("{}", requested.name())));
            rendering::Renderer::Software
        };
        let first_frame = Rc::new(std::cell::Cell::new(None));
        match run(store, renderer, started, first_frame.clone()) {
            Ok(()) => Ok(0),
            Err(error) if rendering::should_retry(renderer, first_frame.get(), &error) => {
                eprintln!("{}", opentermius::tf!("GPU window initialization failed; falling back to software rendering: {error:#}", "error:#" => format!("{:#}", error)));
                rendering::retry_software()
            }
            Err(error) => Err(error),
        }
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            if args.iter().any(|a| a == "--smoke-test" || a == "--benchmark") {
                let output = args.iter().find_map(|a| a.strip_prefix("--output=")).unwrap_or("test-output");
                let _ = std::fs::create_dir_all(output);
                let _ = std::fs::write(PathBuf::from(output).join("error.txt"), format!("{error:#}"));
                eprintln!("{}", opentermius::tf!("OpenTermius failed to start: {error:#}", "error:#" => format!("{:#}", error)));
            } else {
                rfd::MessageDialog::new()
                    .set_title(crate::i18n::t("OpenTermius failed to start"))
                    .set_description(format!("{error:#}"))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
            std::process::exit(1)
        }
    }
}

#[cfg(test)]
mod terminal_ux_tests {
    use super::*;

    #[test]
    fn clipboard_shortcuts_follow_windows_terminal() {
        use ClipboardShortcut::*;
        // Ctrl+C copies only with a selection; without one it stays an interrupt.
        assert_eq!(clipboard_shortcut("c", true, false, false, true), Some(Copy));
        assert_eq!(clipboard_shortcut("\u{3}", true, false, false, true), Some(Copy));
        assert_eq!(clipboard_shortcut("c", true, false, false, false), None);
        assert_eq!(clipboard_shortcut("C", true, false, true, false), Some(Copy));
        assert_eq!(clipboard_shortcut("v", true, false, false, false), Some(Paste));
        assert_eq!(clipboard_shortcut("V", true, false, true, false), Some(Paste));
        assert_eq!(clipboard_shortcut("\u{16}", true, false, false, false), Some(Paste));
        assert_eq!(clipboard_shortcut("\u{f727}", true, false, false, false), Some(Copy));
        assert_eq!(clipboard_shortcut("\u{f727}", false, false, true, false), Some(Paste));
        // Plain keys and Alt chords reach the remote shell untouched.
        assert_eq!(clipboard_shortcut("\u{f727}", false, false, false, false), None);
        assert_eq!(clipboard_shortcut("v", false, false, false, false), None);
        assert_eq!(clipboard_shortcut("v", true, true, false, false), None);
    }

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
            tag_filter: String::new(),
            local_path: PathBuf::new(), local_request: 0, pending_sftp: None, sftp_views: Default::default(), typed_secrets: BTreeMap::new(), server_stats: BTreeMap::new(),
            remote_entries: Vec::new(), local_entries: Vec::new(),
            transfers: BTreeMap::new(), next_transfer: 0,
            render_model: Rc::new(VecModel::from(Vec::new())),
            selecting: false, mouse_button: None,
        }
    }

    fn profile() -> Profile {
        Profile {
            id: "profile".into(), name: "Test SSH".into(), host: "localhost".into(), port: 2222,
            username: "tester".into(), auth_type: "password".into(),
            private_key_path: String::new(), key_id: String::new(), group: "Development".into(), encrypted_secret: None,
            tags: Vec::new(), last_connected: None, created_at: None,
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

    #[cfg(windows)]
    #[test]
    fn typed_credentials_are_saved_but_rejected_ones_are_not() {
        let mut s = state();
        let temp = tempfile::tempdir().unwrap();
        s.store = Arc::new(Mutex::new(Store::open(temp.path().join("config.json")).unwrap()));
        let mut saved = profile();
        saved.username.clear();
        s.store.lock().unwrap().save(saved, None, false).unwrap();
        let secret = |s: &State| {
            let store = s.store.lock().unwrap();
            let host = store.data.hosts.iter().find(|p| p.id == "profile").unwrap().clone();
            (host.username, host.encrypted_secret.map(|e| store::unprotect(&e).unwrap()))
        };

        // Host editor: a typed password is kept even when the draft says not to remember.
        let mut draft = s.edit("profile");
        draft.secret = "editor-secret".into();
        draft.remember = false;
        assert!(s.save(draft));
        assert_eq!(secret(&s).1.as_deref(), Some("editor-secret"));

        // Connect prompt: saved only once the login succeeded, with the typed username.
        let _rx = session(&mut s);
        s.typed_secrets.insert("old".into(), zeroize::Zeroizing::new("wrong-secret".into()));
        s.event(Event::Status { id: "old".into(), connected: false, message: "denied".into(), error: true });
        assert_eq!(secret(&s), (String::new(), Some("editor-secret".into())));
        s.typed_secrets.insert("old".into(), zeroize::Zeroizing::new("prompt-secret".into()));
        s.event(Event::Status { id: "old".into(), connected: true, message: String::new(), error: false });
        assert_eq!(secret(&s), ("tester".into(), Some("prompt-secret".into())));
        assert!(s.typed_secrets.is_empty());
        assert!(!std::fs::read_to_string(temp.path().join("config.json")).unwrap().contains("prompt-secret"));
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
        TEST_CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = Some("pasted".into()));
        s.pointer(0, 1, 2, 3, false, false, false);
        assert!(!s.selecting); // Right-click never begins a text selection.
        // Without a selection it pastes (Windows console convention).
        assert!(matches!(rx.try_recv().unwrap(), Command::Input(data) if data == b"pasted"));
        TEST_CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = None);
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
