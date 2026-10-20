//! Persisted preferences: older configs must keep loading and new values must
//! survive a store round-trip.
use opentermius::{
    settings::{self, Appearance, ColorScheme, Settings},
    store::Store,
    terminal::Terminal,
};

#[test]
fn config_from_before_settings_loads_defaults_and_saves_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("opentermius-native.json");
    std::fs::write(&path, br#"{"version":2,"hosts":[],"knownHosts":{}}"#).unwrap();

    let mut store = Store::open(path.clone()).unwrap();
    assert_eq!(store.data.settings, Settings::default());
    assert_eq!(store.data.settings.font_size, 14);
    assert_eq!(store.data.settings.scrollback_lines, 3000);
    assert!(!store.data.settings.gpu_rendering);

    let updated = Settings {
        font_size: 22,
        copy_on_select: true,
        color_scheme: ColorScheme::Monokai,
        appearance: Appearance::Dark,
        gpu_rendering: true,
        keepalive_seconds: 0,
        ..Settings::default()
    };
    let saved = settings::persist(&mut store, updated.clone()).unwrap();
    assert_eq!(saved, updated);

    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.settings, updated);
    let raw = std::fs::read_to_string(&reopened.path).unwrap();
    assert!(raw.contains("\"fontSize\": 22"));
    assert!(raw.contains("\"colorScheme\": \"monokai\""));
    assert!(raw.contains("\"appearance\": \"dark\""));
    assert!(raw.contains("\"gpuRendering\": true"));
    assert!(raw.contains("\"keepaliveSeconds\": 0"));
}

#[test]
fn appearance_defaults_and_system_resolution() {
    let old: Settings = serde_json::from_str(r#"{"fontSize":18}"#).unwrap();
    assert_eq!(old.appearance, Appearance::Light);
    assert!(!old.gpu_rendering);
    for (index, appearance) in [
        (0, Appearance::Light),
        (1, Appearance::Dark),
        (2, Appearance::System),
    ] {
        assert_eq!(Appearance::from_index(index), appearance);
        assert_eq!(appearance.index(), index);
        let json = serde_json::to_string(&appearance).unwrap();
        assert_eq!(
            serde_json::from_str::<Appearance>(&json).unwrap(),
            appearance
        );
    }
    for system_dark in [false, true] {
        assert!(!Appearance::Light.is_dark(system_dark));
        assert!(Appearance::Dark.is_dark(system_dark));
        assert_eq!(Appearance::System.is_dark(system_dark), system_dark);
    }
}

#[test]
fn out_of_range_config_values_are_normalized_on_save() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("data.json")).unwrap();
    let saved = settings::persist(
        &mut store,
        Settings {
            font_size: 2,
            scrollback_lines: 5,
            connection_timeout_seconds: 0,
            ..Settings::default()
        },
    )
    .unwrap();
    assert_eq!(saved.font_size, 10);
    assert_eq!(saved.scrollback_lines, 1000);
    assert_eq!(saved.connection_timeout_seconds, 1);
    assert_eq!(
        Store::open(store.path.clone()).unwrap().data.settings,
        saved
    );
}

#[test]
fn failed_save_restores_the_previous_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data.json");
    let mut store = Store::open(path.clone()).unwrap();
    let previous = settings::persist(&mut store, Settings::default()).unwrap();
    // A regular file cannot be used as a parent directory. This failure does
    // not depend on filesystem permissions or on which user runs the test.
    store.path = path.join("cannot-write.json");
    assert!(
        settings::persist(
            &mut store,
            Settings {
                font_size: 24,
                ..previous.clone()
            },
        )
        .is_err()
    );
    assert_eq!(store.data.settings, previous);
    assert_eq!(Store::open(path).unwrap().data.settings, previous);
}

#[test]
fn paste_confirmation_counts_crlf_and_trailing_line_breaks() {
    for text in ["", "echo hello"] {
        assert_eq!(settings::paste_line_count(text), 1);
    }
    for text in [
        "echo hello\n",
        "echo hello\r",
        "echo hello\r\n",
        "one\ntwo",
        "one\rtwo",
    ] {
        assert_eq!(settings::paste_line_count(text), 2, "{text:?}");
    }
    assert_eq!(settings::paste_line_count("one\r\ntwo\r\n"), 3);
}

#[test]
fn themes_preserve_inverse_video_and_explicit_rgb_colors() {
    let mut terminal = Terminal::new(4, 30);
    terminal.feed(b"\x1b[7mI\x1b[0m\x1b[38;2;226;147;156mR\x1b[0m\x1b[31mA");
    let snapshot = terminal.snapshot();
    for scheme in ColorScheme::ALL {
        let inverse = snapshot.runs.iter().find(|run| run.text == "I").unwrap();
        assert_eq!(
            scheme.map_color(inverse.fg, inverse.fg_source),
            scheme.background()
        );
        assert_eq!(
            scheme.map_color(inverse.bg, inverse.bg_source),
            scheme.foreground()
        );
        let rgb = snapshot.runs.iter().find(|run| run.text == "R").unwrap();
        assert_eq!(scheme.map_color(rgb.fg, rgb.fg_source), 0xe2939c);
        let ansi = snapshot.runs.iter().find(|run| run.text == "A").unwrap();
        assert_eq!(
            scheme.map_color(ansi.fg, ansi.fg_source),
            scheme.palette()[1]
        );
    }
}

#[test]
fn local_terminal_snapshots_expose_bells_without_losing_them_on_repaint() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"hello\x07");
    assert_eq!(terminal.snapshot().bell_count, 1);
    assert_eq!(terminal.snapshot().bell_count, 1);
    terminal.feed(b"\x1bg");
    assert_eq!(terminal.snapshot().bell_count, 2);
}

#[tokio::test]
async fn ssh_connection_uses_the_configured_handshake_timeout() {
    use opentermius::{ssh, store::Profile};
    use std::time::{Duration, Instant};
    use tokio::{net::TcpListener, sync::mpsc, time::timeout};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Hold TCP open without sending an SSH identification string.
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let (events, mut received) = mpsc::channel(16);
    let started = Instant::now();
    let session = ssh::spawn_with_options(
        &tokio::runtime::Handle::current(),
        "settings-timeout".into(),
        Profile {
            id: "settings-timeout".into(),
            name: "Settings timeout test".into(),
            host: "127.0.0.1".into(),
            port,
            username: "tester".into(),
            auth_type: "password".into(),
            private_key_path: String::new(),
            key_id: String::new(),
            group: String::new(),
            encrypted_secret: None,
            tags: Vec::new(),
            last_connected: None,
            created_at: None,
        },
        String::new(),
        None,
        events,
        ssh::ConnectOptions {
            timeout_seconds: 1,
            keepalive_seconds: 0,
            ..Default::default()
        },
    );
    let failure = timeout(Duration::from_secs(5), async {
        while let Some(event) = received.recv().await {
            if let ssh::Event::Status {
                error: true,
                message,
                ..
            } = event
            {
                return message;
            }
        }
        panic!("SSH closed without reporting its timeout");
    })
    .await;
    server.abort();
    let message = failure.expect("configured one-second timeout was not applied");
    assert!(message.contains("SSH 连接超时"), "{message}");
    assert!(started.elapsed() >= Duration::from_secs(1));
    timeout(Duration::from_secs(2), session.task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn configured_history_limit_applies_to_new_local_parsers() {
    opentermius::terminal::set_default_scrollback(1000);
    let mut terminal = Terminal::new(4, 20);
    opentermius::terminal::set_default_scrollback(opentermius::terminal::DEFAULT_SCROLLBACK);
    for line in 0..1500 {
        terminal.feed(format!("row-{line}\r\n").as_bytes());
    }
    terminal.scroll(10_000);
    assert_eq!(terminal.snapshot().lines[0], "row-497");
}
