//! Wires the Settings page bridge to the persisted preferences.
use crate::{AppWindow, SettingsBridge};
use opentermius::settings::{self, BellMode, ColorScheme, Settings};
use slint::ComponentHandle;

pub fn install(ui: &AppWindow) {
    let settings = current();
    let path = config_path();
    apply_to_ui(ui, &settings, &path);

    let weak = ui.as_weak();
    ui.global::<SettingsBridge>().on_apply(move || {
        let Some(ui) = weak.upgrade() else { return };
        let next = read(&ui);
        if persist(&next) {
            let path = config_path();
            apply_to_ui(&ui, &next, &path);
        } else {
            apply_to_ui(&ui, &current(), &config_path());
        }
    });

    let weak = ui.as_weak();
    ui.global::<SettingsBridge>().on_restore_defaults(move || {
        let Some(ui) = weak.upgrade() else { return };
        let defaults = Settings::default();
        if persist(&defaults) {
            let path = config_path();
            apply_to_ui(&ui, &defaults, &path);
        } else {
            apply_to_ui(&ui, &current(), &config_path());
        }
    });

    ui.global::<SettingsBridge>()
        .on_open_config_folder(open_config_folder);
}

fn current() -> Settings {
    crate::with_state(|state| state.settings.clone()).unwrap_or_default()
}

fn config_path() -> String {
    crate::with_state(|state| {
        state
            .store
            .lock()
            .ok()
            .map(|store| store.path.display().to_string())
    })
    .flatten()
    .unwrap_or_default()
}

fn config_directory() -> Option<std::path::PathBuf> {
    crate::with_state(|state| {
        state
            .store
            .lock()
            .ok()
            .and_then(|store| store.path.parent().map(|path| path.to_path_buf()))
    })
    .flatten()
}

fn read(ui: &AppWindow) -> Settings {
    let bridge = ui.global::<SettingsBridge>();
    let mut settings = Settings {
        font_size: bridge
            .get_terminal_font_size()
            .clamp(0, i32::from(u16::MAX)) as u16,
        scrollback_lines: bridge.get_scrollback_lines().max(0) as usize,
        cursor_blink: bridge.get_cursor_blink(),
        copy_on_select: bridge.get_copy_on_select(),
        confirm_multiline_paste: bridge.get_confirm_multiline_paste(),
        bell: BellMode::from_index(bridge.get_bell_mode()),
        color_scheme: ColorScheme::from_index(bridge.get_color_scheme()),
        keepalive_seconds: bridge.get_keepalive_seconds().max(0) as u32,
        connection_timeout_seconds: bridge.get_connection_timeout_seconds().max(0) as u32,
        confirm_overwrite: bridge.get_confirm_overwrite(),
        show_hidden_files: bridge.get_show_hidden_files(),
    };
    settings.sanitize();
    settings
}

fn apply_to_ui(ui: &AppWindow, settings: &Settings, config_path: &str) {
    opentermius::terminal::set_default_scrollback(settings.scrollback_lines);
    let bridge = ui.global::<SettingsBridge>();
    bridge.set_terminal_font_size(i32::from(settings.font_size));
    bridge.set_scrollback_lines(settings.scrollback_lines.min(i32::MAX as usize) as i32);
    bridge.set_cursor_blink(settings.cursor_blink);
    bridge.set_copy_on_select(settings.copy_on_select);
    bridge.set_confirm_multiline_paste(settings.confirm_multiline_paste);
    bridge.set_bell_mode(settings.bell.index());
    bridge.set_color_scheme(settings.color_scheme.index());
    bridge.set_keepalive_seconds(settings.keepalive_seconds.min(i32::MAX as u32) as i32);
    bridge.set_connection_timeout_seconds(
        settings.connection_timeout_seconds.min(i32::MAX as u32) as i32
    );
    bridge.set_confirm_overwrite(settings.confirm_overwrite);
    bridge.set_show_hidden_files(settings.show_hidden_files);
    bridge.set_version(settings::VERSION.into());
    bridge.set_config_path(config_path.into());

    let scheme = settings.color_scheme;
    ui.set_terminal_background(crate::ui_color(scheme.background()));
    ui.set_terminal_cursor(color_with_alpha(scheme.cursor(), 140));
    if settings.bell == BellMode::None {
        ui.set_bell_flash(false);
    }
}

fn color_with_alpha(rgb: u32, alpha: u8) -> slint::Color {
    slint::Color::from_argb_u8(alpha, (rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn persist(settings: &Settings) -> bool {
    crate::with_state(|state| {
        let result = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("配置锁不可用"))
            .and_then(|mut store| settings::persist(&mut store, settings.clone()));
        match result {
            Ok(saved) => {
                let refresh_files = state.settings.show_hidden_files != saved.show_hidden_files;
                state.settings = saved;
                state.display();
                if refresh_files {
                    state.list_local(String::new());
                    if state.remote_ready() {
                        state.list_files(state.file_path.clone());
                    }
                }
                state.status("设置已保存", false);
                true
            }
            Err(error) => {
                state.status(error.to_string(), true);
                false
            }
        }
    })
    .unwrap_or(false)
}

fn open_config_folder() {
    let Some(directory) = config_directory() else {
        return;
    };
    let _ = std::fs::create_dir_all(&directory);
    #[cfg(windows)]
    let result = std::process::Command::new("explorer")
        .arg(&directory)
        .spawn();
    #[cfg(not(windows))]
    let result = std::process::Command::new("xdg-open")
        .arg(&directory)
        .spawn();
    if let Err(error) = result {
        crate::with_state(|state| state.status(format!("无法打开配置目录：{error}"), true));
    }
}
