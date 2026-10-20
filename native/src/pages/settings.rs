//! Wires the Settings page bridge to the persisted preferences.
use crate::{AppWindow, SettingsBridge, Theme};
use opentermius::settings::{self, Appearance, BellMode, ColorScheme, Settings};
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

pub(super) fn reload(ui: &AppWindow) {
    let saved = crate::with_state(|state| {
        state
            .store
            .lock()
            .ok()
            .map(|store| store.data.settings.clone().normalized())
    })
    .flatten();
    if let Some(saved) = saved {
        crate::with_state(|state| state.settings = saved.clone());
        apply_to_ui(ui, &saved, &config_path());
        crate::with_state(|state| {
            state.display();
            state.render_files();
            state.list_local(String::new());
        });
    }
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
        appearance: Appearance::from_index(bridge.get_appearance()),
        gpu_rendering: bridge.get_gpu_rendering(),
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
    bridge.set_appearance(settings.appearance.index());
    bridge.set_gpu_rendering(settings.gpu_rendering);
    ui.global::<Theme>()
        .set_dark(settings.appearance.is_dark(settings::system_dark()));
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
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg(&directory)
        .spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(&directory)
        .spawn();
    if let Err(error) = result {
        crate::with_state(|state| state.status(format!("无法打开配置目录：{error}"), true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
    };
    use std::rc::Rc;

    struct TestPlatform(Rc<MinimalSoftwareWindow>);
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }

    fn click(ui: &AppWindow, x: f32, y: f32) {
        slint::platform::update_timers_and_animations();
        let position = slint::LogicalPosition::new(x, y);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }

    #[test]
    fn appearance_callbacks_navigation_and_terminal_metrics() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
        window.set_size(slint::PhysicalSize::new(1240, 800));
        let ui = AppWindow::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let mut store = opentermius::store::Store::open(path.clone()).unwrap();
        let saved = Settings {
            appearance: Appearance::Dark,
            ..Settings::default()
        };
        settings::persist(&mut store, saved.clone()).unwrap();
        let mut state = crate::terminal_ux_tests::state();
        state.ui = ui.as_weak();
        state.store = std::sync::Arc::new(std::sync::Mutex::new(store));
        state.settings = saved;
        crate::STATE.with(|slot| *slot.borrow_mut() = Some(state));
        install(&ui);
        ui.show().unwrap();
        assert!(
            ui.global::<Theme>().get_dark(),
            "saved appearance applies at startup"
        );

        let bridge = ui.global::<SettingsBridge>();
        assert!(!bridge.get_gpu_rendering());
        ui.invoke_open_section("settings".into());
        // Settings uses main's 720px column; click the GPU switch in Appearance.
        click(&ui, 887., 235.);
        assert!(current().gpu_rendering);
        assert!(opentermius::store::Store::open(path.clone()).unwrap().data.settings.gpu_rendering);
        click(&ui, 887., 235.);
        assert!(!current().gpu_rendering);
        click(&ui, 887., 235.);
        assert!(current().gpu_rendering);
        for appearance in [Appearance::Light, Appearance::Dark, Appearance::System] {
            bridge.set_appearance(appearance.index());
            bridge.invoke_apply();
            assert_eq!(
                ui.global::<Theme>().get_dark(),
                appearance.is_dark(settings::system_dark())
            );
            assert_eq!(
                opentermius::store::Store::open(path.clone())
                    .unwrap()
                    .data
                    .settings
                    .appearance,
                appearance
            );
        }

        // The SFTP toolbar and Settings share the same persisted hidden-file switch.
        crate::with_state(|state| {
            state.remote_entries = [".hidden", "visible"]
                .into_iter()
                .map(|name| opentermius::ssh::RemoteFile {
                    name: name.into(),
                    path: format!("/{name}"),
                    size: 1,
                    is_dir: false,
                    modified: "0".into(),
                    permissions: None,
                })
                .collect();
        });
        bridge.set_show_hidden_files(false);
        bridge.invoke_apply();
        crate::with_state(|state| state.render_files());
        assert!(!ui.get_show_hidden());
        assert_eq!(slint::Model::row_count(&ui.get_files()), 1);
        ui.set_show_hidden(true);
        bridge.invoke_apply();
        crate::with_state(|state| state.render_files());
        assert!(current().show_hidden_files);
        assert_eq!(slint::Model::row_count(&ui.get_files()), 2);

        // Both visible Settings entries must navigate away from a terminal.
        ui.invoke_activate_session("test".into());
        ui.set_menu_open(true);
        click(&ui, 70., 231.);
        assert!(!ui.get_menu_open());
        assert_eq!(ui.get_section(), "settings");
        ui.invoke_open_section("hosts".into());
        ui.invoke_activate_session("test".into());
        click(&ui, 70., 744.);
        assert_eq!(ui.get_section(), "settings");

        ui.invoke_activate_session("test".into());
        slint::platform::update_timers_and_animations();
        let width = ui.get_cell_width();
        let height = ui.get_cell_height();
        let cols = ui.get_terminal_cols();
        let rows = ui.get_terminal_rows();
        bridge.set_terminal_font_size(22);
        bridge.invoke_apply();
        slint::platform::update_timers_and_animations();
        assert!((ui.get_cell_width() / width - 22. / 14.).abs() < 0.05);
        assert!(ui.get_cell_height() > height);
        assert!(ui.get_terminal_cols() < cols);
        assert!(ui.get_terminal_rows() < rows);

        // Recovery changes the store directly; reload must refresh live preferences.
        crate::with_state(|state| {
            let mut store = state.store.lock().unwrap();
            settings::persist(
                &mut store,
                Settings {
                    appearance: Appearance::Dark,
                    font_size: 18,
                    ..Settings::default()
                },
            )
            .unwrap();
        });
        reload(&ui);
        assert!(ui.global::<Theme>().get_dark());
        assert_eq!(bridge.get_terminal_font_size(), 18);
        assert_eq!(current().font_size, 18);

        bridge.invoke_restore_defaults();
        assert!(!ui.global::<Theme>().get_dark());
        assert_eq!(bridge.get_terminal_font_size(), 14);
        assert!(!bridge.get_gpu_rendering());
        assert!(!opentermius::store::Store::open(path).unwrap().data.settings.gpu_rendering);
        ui.hide().unwrap();
        crate::STATE.with(|slot| *slot.borrow_mut() = None);
    }
}
