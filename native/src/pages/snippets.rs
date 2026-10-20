use crate::{AppWindow, SnippetRow, SnippetsBridge, with_state};
use opentermius::{
    snippets::{self, Snippet},
    ssh::Command,
};
use slint::{ComponentHandle, SharedString};

fn row(snippet: &Snippet) -> SnippetRow {
    SnippetRow {
        id: snippet.id.clone().into(),
        name: snippet.name.clone().into(),
        preview: snippets::first_line(&snippet.script).into(),
        tags: snippet.tags.join(" · ").into(),
        updated: snippets::format_timestamp(snippet.updated_at).into(),
    }
}

fn refresh(ui: &AppWindow, query: &str) {
    let rows = with_state(|s| {
        s.store.lock().ok().map(|store| {
            snippets::search(&store.data.snippets, query)
                .into_iter()
                .map(row)
                .collect::<Vec<_>>()
        })
    })
    .flatten()
    .unwrap_or_default();
    ui.global::<SnippetsBridge>()
        .set_snippets(crate::model(rows));
}

fn close_editor(ui: &AppWindow) {
    let bridge = ui.global::<SnippetsBridge>();
    bridge.set_editing(false);
    bridge.set_draft_id("".into());
    bridge.set_draft_name("".into());
    bridge.set_draft_script("".into());
    bridge.set_draft_tags("".into());
    bridge.set_error_text("".into());
}

fn show_error(ui: &AppWindow, message: &str) {
    ui.global::<SnippetsBridge>().set_error_text(message.into());
    with_state(|s| s.status(message, true));
}

/// Mutate the snippet list and persist it. On validation or I/O failure the
/// in-memory list is restored so the UI never shows unsaved state.
fn persist(mutate: impl FnOnce(&mut Vec<Snippet>) -> anyhow::Result<()>) -> anyhow::Result<()> {
    with_state(|s| {
        let mut store = s
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("配置锁不可用"))?;
        let previous = store.data.snippets.clone();
        if let Err(error) = mutate(&mut store.data.snippets) {
            store.data.snippets = previous;
            return Err(error);
        }
        if let Err(error) = store.persist() {
            store.data.snippets = previous;
            return Err(error);
        }
        Ok(())
    })
    .unwrap_or_else(|| Err(anyhow::anyhow!("界面状态不可用")))
}

pub fn install(ui: &AppWindow) {
    {
        let bridge = ui.global::<SnippetsBridge>();
        bridge.set_query("".into());
        bridge.set_editing(false);
        bridge.set_draft_id("".into());
        bridge.set_draft_name("".into());
        bridge.set_draft_script("".into());
        bridge.set_draft_tags("".into());
        bridge.set_error_text("".into());
    }
    refresh(ui, "");

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>().on_search(move |query| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<SnippetsBridge>().set_query(query.clone());
            refresh(&ui, &query);
        }
    });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>().on_create(move || {
        if let Some(ui) = weak.upgrade() {
            let bridge = ui.global::<SnippetsBridge>();
            bridge.set_editing(true);
            bridge.set_draft_id("".into());
            bridge.set_draft_name("".into());
            bridge.set_draft_script("".into());
            bridge.set_draft_tags("".into());
            bridge.set_error_text("".into());
        }
    });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>().on_edit(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let snippet = with_state(|s| {
            s.store
                .lock()
                .ok()
                .and_then(|store| snippets::find(&store.data.snippets, &id).cloned())
        })
        .flatten();
        let Some(snippet) = snippet else {
            with_state(|s| s.status("片段不存在", true));
            return;
        };
        let bridge = ui.global::<SnippetsBridge>();
        bridge.set_editing(true);
        bridge.set_draft_id(snippet.id.into());
        bridge.set_draft_name(snippet.name.into());
        bridge.set_draft_script(snippet.script.into());
        bridge.set_draft_tags(snippet.tags.join(", ").into());
        bridge.set_error_text("".into());
    });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>().on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            close_editor(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>()
        .on_save(move |name, tags, script| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let draft_id = ui.global::<SnippetsBridge>().get_draft_id().to_string();
            let query = ui.global::<SnippetsBridge>().get_query().to_string();
            let draft = Snippet {
                id: draft_id,
                name: name.to_string(),
                script: script.to_string(),
                tags: snippets::parse_tags(&tags),
                ..Snippet::default()
            };
            let now = chrono::Utc::now().timestamp_millis();
            match persist(|list| snippets::save(list, draft, now).map(|_| ())) {
                Ok(()) => {
                    close_editor(&ui);
                    refresh(&ui, &query);
                    with_state(|s| s.status("已保存片段", false));
                }
                Err(error) => show_error(&ui, &error.to_string()),
            }
        });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>().on_delete(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let query = ui.global::<SnippetsBridge>().get_query().to_string();
        match persist(|list| {
            if snippets::delete(list, &id) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("片段不存在"))
            }
        }) {
            Ok(()) => {
                close_editor(&ui);
                refresh(&ui, &query);
                with_state(|s| s.status("已删除片段", false));
            }
            Err(error) => show_error(&ui, &error.to_string()),
        }
    });

    let weak = ui.as_weak();
    ui.global::<SnippetsBridge>()
        .on_run(move |id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let session = with_state(|s| {
                let snippet = match s.store.lock() {
                    Ok(store) => snippets::find(&store.data.snippets, &id).cloned(),
                    Err(_) => {
                        s.status("配置锁不可用", true);
                        return None;
                    }
                };
                let Some(snippet) = snippet else {
                    s.status("片段不存在", true);
                    return None;
                };
                let Some(session_id) = s.active.clone() else {
                    s.status("请先打开终端会话", true);
                    return None;
                };
                let Some(bracketed) = s
                    .sessions
                    .get(&session_id)
                    .map(|view| view.snapshot.bracketed_paste)
                else {
                    s.status("请先打开终端会话", true);
                    return None;
                };
                s.send(Command::Input(snippets::to_terminal_input(
                    &snippet.script,
                    bracketed,
                )));
                Some(session_id)
            })
            .flatten();
            let Some(session_id) = session else {
                return;
            };
            ui.invoke_activate_session(session_id.into());
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{Platform, WindowAdapter, software_renderer::*};
    use std::{cell::RefCell, rc::Rc};

    struct SoftwarePlatform(Rc<MinimalSoftwareWindow>);

    impl Platform for SoftwarePlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn activating_the_same_session_returns_from_snippets_to_terminal() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(SoftwarePlatform(window))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1240, 800));
        ui.show().unwrap();
        ui.set_active_session("active".into());
        ui.set_terminal_visible(true);

        let selected = Rc::new(RefCell::new(String::new()));
        let captured = selected.clone();
        let weak = ui.as_weak();
        ui.on_select_session(move |id| {
            *captured.borrow_mut() = id.to_string();
            weak.upgrade().unwrap().set_active_session(id);
        });
        slint::platform::update_timers_and_animations();
        ui.invoke_open_section("snippets".into());
        slint::platform::update_timers_and_animations();
        let page = ui.window().take_snapshot().unwrap();
        let page_pixel = page.as_slice()[650 * 1240 + 1000];
        assert_eq!((page_pixel.r, page_pixel.g, page_pixel.b), (237, 241, 242));

        if let Some(output) = std::env::var_os("OPENTERMIUS_SNIPPETS_SCREENSHOTS") {
            let bridge = ui.global::<SnippetsBridge>();
            bridge.set_snippets(crate::model(vec![SnippetRow {
                id: "deploy".into(),
                name: "Deploy staging".into(),
                preview: "cd /srv/application".into(),
                tags: "deploy · staging".into(),
                updated: "2026-10-20 12:00".into(),
            }]));
            bridge.set_editing(true);
            bridge.set_draft_id("deploy".into());
            bridge.set_draft_name("Deploy staging".into());
            bridge.set_draft_script("cd /srv/application\n./deploy.sh staging".into());
            bridge.set_draft_tags("deploy, staging".into());
            slint::platform::update_timers_and_animations();
            crate::capture(&ui, &std::path::PathBuf::from(output)).unwrap();
        }

        // The session id stays unchanged, so navigation must not depend on
        // the active-session property's changed handler.
        ui.invoke_activate_session("active".into());
        slint::platform::update_timers_and_animations();
        assert_eq!(&*selected.borrow(), "active");
        assert_eq!(ui.get_active_session(), "active");
        assert!(!ui.get_sftp_page());
        let terminal = ui.window().take_snapshot().unwrap();
        let terminal_pixel = terminal.as_slice()[650 * 1240 + 1000];
        assert_eq!(
            (terminal_pixel.r, terminal_pixel.g, terminal_pixel.b),
            (20, 23, 41)
        );
        ui.hide().unwrap();
    }
}
