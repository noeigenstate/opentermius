//! User preferences persisted with the local configuration.
//!
//! Every value is clamped to the range offered by the settings page. The module
//! also owns the terminal color schemes so rendering can be changed without
//! touching the SSH/SFTP transport.
use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_FG: u32 = 0xffffff;
pub const DEFAULT_BG: u32 = 0x141729;
pub const DEFAULT_SCROLLBACK: usize = 3000;
pub const ANSI_COLORS: [u32; 16] = [
    0x1a202a, 0xe2939c, 0x92ccac, 0xe5c58b, 0x8baedb, 0xb7a0d7, 0x84c9cf, 0xd3deec, 0x687c93,
    0xf3a9b1, 0xb2e3bf, 0xf0d6ab, 0xaccaf0, 0xd3baed, 0xa5e0df, 0xf2f6fc,
];

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum ColorSource {
    DefaultForeground,
    DefaultBackground,
    Ansi(u8),
    Explicit,
}

/// Product version shown on the About card.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const FONT_SIZE_MIN: u16 = 10;
pub const FONT_SIZE_MAX: u16 = 24;
pub const DEFAULT_FONT_SIZE: u16 = 14;
pub const SCROLLBACK_MIN: usize = 1000;
pub const SCROLLBACK_MAX: usize = 100_000;
pub const KEEPALIVE_DEFAULT: u32 = 30;
pub const KEEPALIVE_MAX: u32 = 3600;
pub const CONNECTION_TIMEOUT_DEFAULT: u32 = 15;
pub const CONNECTION_TIMEOUT_MIN: u32 = 1;
pub const CONNECTION_TIMEOUT_MAX: u32 = 300;
pub const THEME_DAY_START: &str = "07:00";
pub const THEME_NIGHT_START: &str = "19:00";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    #[default]
    Light,
    Dark,
    System,
}

impl Appearance {
    pub const fn index(self) -> i32 {
        match self {
            Self::Light => 0,
            Self::Dark => 1,
            Self::System => 2,
        }
    }

    pub const fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Dark,
            2 => Self::System,
            _ => Self::Light,
        }
    }

    pub const fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => system_dark,
        }
    }
}

impl<'de> Deserialize<'de> for Appearance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Self::Dark,
            "system" => Self::System,
            _ => Self::Light,
        })
    }
}

/// Resolve Windows app appearance; unavailable preferences default to light.
pub fn system_dark() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW,
        };
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
            .encode_utf16()
            .collect();
        let name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut value = 1u32;
        let mut size = std::mem::size_of_val(&value) as u32;
        // The registry API writes at most `size` bytes into this DWORD buffer.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut value as *mut u32).cast(),
                &mut size,
            )
        };
        result == 0 && value == 0
    }
    #[cfg(not(windows))]
    false
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    #[default]
    Default,
    SolarizedDark,
    Monokai,
    Light,
}

impl ColorScheme {
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::SolarizedDark,
        Self::Monokai,
        Self::Light,
    ];

    pub const fn index(self) -> i32 {
        match self {
            Self::Default => 0,
            Self::SolarizedDark => 1,
            Self::Monokai => 2,
            Self::Light => 3,
        }
    }

    pub fn from_index(index: i32) -> Self {
        Self::ALL[index.clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => crate::i18n::t("Match app theme"),
            Self::SolarizedDark => crate::i18n::t("Solarized Dark"),
            Self::Monokai => crate::i18n::t("Monokai"),
            Self::Light => crate::i18n::t("Light"),
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::SolarizedDark => "solarized-dark",
            Self::Monokai => "monokai",
            Self::Light => "light",
        }
    }

    pub fn from_key(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "solarized-dark" | "solarized_dark" | "solarizeddark" => Self::SolarizedDark,
            "monokai" => Self::Monokai,
            "light" => Self::Light,
            _ => Self::Default,
        }
    }

    /// `Default` follows the app: the dark terminal in the dark theme, the light one otherwise.
    pub fn resolve(self, dark: bool) -> Self {
        if self == Self::Default && !dark { Self::Light } else { self }
    }

    pub const fn background(self) -> u32 {
        match self {
            // The app's dark content surface, so the terminal blends into the window.
            Self::Default => 0x1b1e2e,
            Self::SolarizedDark => 0x002b36,
            Self::Monokai => 0x272822,
            Self::Light => 0xffffff,
        }
    }

    pub const fn foreground(self) -> u32 {
        match self {
            Self::Default => 0xe6e9f2,
            Self::SolarizedDark => 0x839496,
            Self::Monokai => 0xf8f8f2,
            Self::Light => 0x141729,
        }
    }

    pub const fn cursor(self) -> u32 {
        match self {
            Self::Default => 0xffffff,
            Self::SolarizedDark => 0x93a1a1,
            Self::Monokai => 0xf8f8f2,
            Self::Light => 0x141729,
        }
    }

    pub const fn palette(self) -> [u32; 16] {
        match self {
            Self::Default => crate::settings::ANSI_COLORS,
            Self::SolarizedDark => [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ],
            Self::Monokai => [
                0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
            ],
            Self::Light => [
                0x141729, 0xc91b00, 0x0f7b0f, 0x9a6700, 0x005cc5, 0xa626a4, 0x0087af, 0xd5dde0,
                0x555b6e, 0xd1242f, 0x1a7f37, 0xbf8700, 0x2188ff, 0xc45ad6, 0x00a6b8, 0xffffff,
            ],
        }
    }

    /// Preserve default and ANSI color identity (including inverse video),
    /// leaving explicit RGB and extended indexed colors intact.
    pub fn map_color(self, value: u32, source: crate::settings::ColorSource) -> u32 {
        match source {
            ColorSource::DefaultForeground => self.foreground(),
            ColorSource::DefaultBackground => self.background(),
            ColorSource::Ansi(index) if index < 16 => self.palette()[index as usize],
            _ => value,
        }
    }
}

impl<'de> Deserialize<'de> for ColorScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Unknown schemes in a hand-edited or future config fall back to the
        // default instead of rejecting the whole file.
        Ok(Self::from_key(&String::deserialize(deserializer)?))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BellMode {
    None,
    #[default]
    Visual,
}

impl BellMode {
    pub const fn index(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Visual => 1,
        }
    }

    pub fn from_index(index: i32) -> Self {
        if index <= 0 { Self::None } else { Self::Visual }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => crate::i18n::t("None"),
            Self::Visual => crate::i18n::t("Visual"),
        }
    }
}

impl<'de> Deserialize<'de> for BellMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.trim().to_ascii_lowercase().as_str() {
            "visual" | "on" => Self::Visual,
            _ => Self::None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub language: crate::i18n::Language,
    pub font_size: u16,
    pub scrollback_lines: usize,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub confirm_multiline_paste: bool,
    pub bell: BellMode,
    pub color_scheme: ColorScheme,
    pub appearance: Appearance,
    pub theme_schedule: bool,
    pub theme_day_start: String,
    pub theme_night_start: String,
    pub gpu_rendering: bool,
    pub keepalive_seconds: u32,
    pub connection_timeout_seconds: u32,
    pub confirm_overwrite: bool,
    pub show_hidden_files: bool,
    /// Check GitHub Releases for a newer version shortly after startup.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: crate::i18n::Language::default(),
            font_size: DEFAULT_FONT_SIZE,
            scrollback_lines: crate::settings::DEFAULT_SCROLLBACK,
            cursor_blink: true,
            copy_on_select: false,
            confirm_multiline_paste: true,
            bell: BellMode::Visual,
            color_scheme: ColorScheme::Default,
            appearance: Appearance::default(),
            theme_schedule: false,
            theme_day_start: THEME_DAY_START.into(),
            theme_night_start: THEME_NIGHT_START.into(),
            gpu_rendering: false,
            keepalive_seconds: KEEPALIVE_DEFAULT,
            connection_timeout_seconds: CONNECTION_TIMEOUT_DEFAULT,
            confirm_overwrite: true,
            show_hidden_files: true,
            auto_update: true,
        }
    }
}

impl Settings {
    pub fn is_dark(&self, system_dark: bool) -> bool {
        use chrono::Timelike;
        if !self.theme_schedule {
            return self.appearance.is_dark(system_dark);
        }
        let now = chrono::Local::now();
        schedule_is_dark(&self.theme_day_start, &self.theme_night_start, now.hour() * 60 + now.minute())
    }

    /// Clamp persisted or user-entered values to the ranges exposed by the UI.
    pub fn sanitize(&mut self) {
        if theme_minutes(&self.theme_day_start).is_none()
            || theme_minutes(&self.theme_night_start).is_none()
            || self.theme_day_start == self.theme_night_start
        {
            self.theme_day_start = THEME_DAY_START.into();
            self.theme_night_start = THEME_NIGHT_START.into();
        }
        self.font_size = self.font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        self.scrollback_lines = self.scrollback_lines.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX);
        self.keepalive_seconds = self.keepalive_seconds.min(KEEPALIVE_MAX);
        self.connection_timeout_seconds = self
            .connection_timeout_seconds
            .clamp(CONNECTION_TIMEOUT_MIN, CONNECTION_TIMEOUT_MAX);
    }

    pub fn normalized(mut self) -> Self {
        self.sanitize();
        self
    }
}

fn theme_minutes(value: &str) -> Option<u32> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':'
        || ![bytes[0], bytes[1], bytes[3], bytes[4]].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let hours = u32::from(bytes[0] - b'0') * 10 + u32::from(bytes[1] - b'0');
    let minutes = u32::from(bytes[3] - b'0') * 10 + u32::from(bytes[4] - b'0');
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

fn schedule_is_dark(day: &str, night: &str, minutes: u32) -> bool {
    let (day, night) = match (theme_minutes(day), theme_minutes(night)) {
        (Some(day), Some(night)) if day != night => (day, night),
        _ => (7 * 60, 19 * 60),
    };
    if day < night { minutes < day || minutes >= night } else { minutes < day && minutes >= night }
}

/// Persist settings with rollback, returning the normalized values that were
/// actually written. Configs created before settings existed simply load the
/// defaults through serde's container-level `default`.
pub fn persist(
    store: &mut crate::store::Store,
    mut settings: Settings,
) -> anyhow::Result<Settings> {
    settings.sanitize();
    let previous = std::mem::replace(&mut store.data.settings, settings.clone());
    if let Err(error) = store.persist() {
        store.data.settings = previous;
        return Err(error);
    }
    Ok(settings)
}

/// Dotfiles are hidden on both local and remote listings. Windows hidden
/// attribute checks live next to the metadata reader in the UI layer.
pub fn hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Include trailing line breaks, since these can execute a pasted command.
pub fn paste_line_count(text: &str) -> usize {
    text.replace("\r\n", "\n").split(['\r', '\n']).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_ranges() {
        let settings = Settings::default();
        assert_eq!(settings.font_size, 14);
        assert_eq!(
            settings.scrollback_lines,
            crate::settings::DEFAULT_SCROLLBACK
        );
        assert_eq!(settings.scrollback_lines, 3000);
        assert!(settings.cursor_blink);
        assert!(!settings.copy_on_select);
        assert!(settings.confirm_multiline_paste);
        assert_eq!(settings.bell, BellMode::Visual);
        assert_eq!(settings.color_scheme, ColorScheme::Default);
        assert!(!settings.gpu_rendering);
        assert_eq!(settings.keepalive_seconds, 30);
        assert_eq!(settings.connection_timeout_seconds, 15);
        assert!(settings.confirm_overwrite);
        assert!(settings.show_hidden_files);
    }

    #[test]
    fn sanitize_clamps_every_numeric_setting() {
        let mut settings = Settings {
            font_size: 1,
            scrollback_lines: 0,
            keepalive_seconds: 99_999,
            connection_timeout_seconds: 0,
            ..Settings::default()
        };
        settings.sanitize();
        assert_eq!(settings.font_size, FONT_SIZE_MIN);
        assert_eq!(settings.scrollback_lines, SCROLLBACK_MIN);
        assert_eq!(settings.keepalive_seconds, KEEPALIVE_MAX);
        assert_eq!(settings.connection_timeout_seconds, CONNECTION_TIMEOUT_MIN);

        let mut settings = Settings {
            font_size: 99,
            scrollback_lines: usize::MAX,
            connection_timeout_seconds: 99_999,
            ..Settings::default()
        };
        settings.sanitize();
        assert_eq!(settings.font_size, FONT_SIZE_MAX);
        assert_eq!(settings.scrollback_lines, SCROLLBACK_MAX);
        assert_eq!(settings.connection_timeout_seconds, CONNECTION_TIMEOUT_MAX);
    }

    #[test]
    fn old_config_without_settings_loads_defaults() {
        let data: crate::store::Data = serde_json::from_str(r#"{"version":2,"hosts":[]}"#).unwrap();
        assert_eq!(data.settings, Settings::default());
        assert_eq!(data.settings.scrollback_lines, 3000);
    }

    #[test]
    fn old_config_with_glass_effect_preserves_other_settings() {
        let source = r#"{
            "version": 2,
            "hosts": [],
            "settings": {
                "language": "zh-CN",
                "fontSize": 20,
                "scrollbackLines": 20000,
                "cursorBlink": false,
                "copyOnSelect": true,
                "confirmMultilinePaste": false,
                "bell": "none",
                "colorScheme": "monokai",
                "appearance": "dark",
                "gpuRendering": true,
                "glassEffect": false,
                "keepaliveSeconds": 0,
                "connectionTimeoutSeconds": 45,
                "confirmOverwrite": false,
                "showHiddenFiles": false,
                "autoUpdate": false
            }
        }"#;
        let expected = Settings {
            language: crate::i18n::Language::Chinese,
            font_size: 20,
            scrollback_lines: 20_000,
            cursor_blink: false,
            copy_on_select: true,
            confirm_multiline_paste: false,
            bell: BellMode::None,
            color_scheme: ColorScheme::Monokai,
            appearance: Appearance::Dark,
            gpu_rendering: true,
            keepalive_seconds: 0,
            connection_timeout_seconds: 45,
            confirm_overwrite: false,
            show_hidden_files: false,
            auto_update: false,
            ..Settings::default()
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.json");
        let mut legacy: serde_json::Value = serde_json::from_str(source).unwrap();
        for glass in [false, true] {
            legacy["settings"]["glassEffect"] = glass.into();
            std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
            let mut store = crate::store::Store::open(path.clone()).unwrap();
            assert_eq!(store.data.settings, expected);
            assert_eq!(persist(&mut store, expected.clone()).unwrap(), expected);
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let mut remaining = legacy["settings"].clone();
            remaining.as_object_mut().unwrap().remove("glassEffect");
            remaining["themeSchedule"] = false.into();
            remaining["themeDayStart"] = THEME_DAY_START.into();
            remaining["themeNightStart"] = THEME_NIGHT_START.into();
            assert_eq!(saved["settings"], remaining);
            assert_eq!(
                crate::store::Store::open(path.clone()).unwrap().data.settings,
                expected
            );
        }
    }

    #[test]
    fn partial_and_unknown_settings_load_defaults() {
        let partial: Settings = serde_json::from_str(r#"{"fontSize":18,"bell":"none"}"#).unwrap();
        assert_eq!(partial.font_size, 18);
        assert_eq!(partial.bell, BellMode::None);
        assert_eq!(partial.scrollback_lines, 3000);
        assert_eq!(partial.keepalive_seconds, 30);

        let unknown: Settings =
            serde_json::from_str(r#"{"colorScheme":"neon","bell":"chime","keepaliveSeconds":10}"#)
                .unwrap();
        assert_eq!(unknown.color_scheme, ColorScheme::Default);
        assert_eq!(unknown.bell, BellMode::None);
        assert_eq!(unknown.keepalive_seconds, 10);
    }

    #[test]
    fn settings_round_trip_uses_camel_case() {
        let settings = Settings {
            font_size: 20,
            scrollback_lines: 20_000,
            color_scheme: ColorScheme::SolarizedDark,
            keepalive_seconds: 0,
            confirm_overwrite: false,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        for key in [
            "fontSize",
            "scrollbackLines",
            "confirmOverwrite",
            "showHiddenFiles",
            "connectionTimeoutSeconds",
            "colorScheme",
            "themeSchedule",
            "themeDayStart",
            "themeNightStart",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn schedule_defaults_sanitizes_and_round_trips() {
        let old: Settings = serde_json::from_str(r#"{"appearance":"system"}"#).unwrap();
        assert!(!old.theme_schedule);
        assert_eq!(old.theme_day_start, THEME_DAY_START);
        assert_eq!(old.theme_night_start, THEME_NIGHT_START);
        for (day, night) in [("", "19:00"), ("24:00", "19:00"), ("07:00", "12:60"), ("7:00", "19:00"), ("07:00", "07:00"), ("07:00", "12:0０")] {
            let mut settings = Settings { theme_day_start: day.into(), theme_night_start: night.into(), ..Settings::default() };
            settings.sanitize();
            assert_eq!(settings.theme_day_start, THEME_DAY_START);
            assert_eq!(settings.theme_night_start, THEME_NIGHT_START);
        }
        let settings = Settings { appearance: Appearance::System, theme_schedule: true, theme_day_start: "20:00".into(), theme_night_start: "06:00".into(), ..Settings::default() };
        let temp = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(temp.path().join("data.json")).unwrap();
        assert_eq!(persist(&mut store, settings.clone()).unwrap(), settings);
        assert_eq!(crate::store::Store::open(store.path.clone()).unwrap().data.settings, settings);
        assert_eq!(serde_json::to_string(&settings.appearance).unwrap(), "\"system\"");
    }

    #[test]
    fn scheduled_settings_remain_readable_with_legacy_appearance_values() {
        #[derive(Debug, PartialEq, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum LegacyAppearance { Light, Dark, System }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LegacySettings { appearance: LegacyAppearance, font_size: u16 }

        for (appearance, legacy, key) in [
            (Appearance::Light, LegacyAppearance::Light, "light"),
            (Appearance::Dark, LegacyAppearance::Dark, "dark"),
            (Appearance::System, LegacyAppearance::System, "system"),
        ] {
            let settings = Settings { appearance, theme_schedule: true, font_size: 20, ..Settings::default() };
            let json = serde_json::to_string(&settings).unwrap();
            let saved: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(saved["appearance"], key);
            assert_ne!(saved["appearance"], "schedule");
            assert_eq!(saved["themeSchedule"], true);
            let old: LegacySettings = serde_json::from_str(&json).unwrap();
            assert_eq!(old.appearance, legacy);
            assert_eq!(old.font_size, 20);
        }
    }

    #[test]
    fn unknown_appearance_loads_as_light_and_preserves_other_settings() {
        let expected = Settings { font_size: 20, theme_schedule: true, theme_day_start: "08:30".into(), theme_night_start: "20:15".into(), ..Settings::default() };
        let mut data = serde_json::json!({ "version": 2, "hosts": [], "settings": expected });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.json");
        for value in ["sepia", "schedule"] {
            data["settings"]["appearance"] = value.into();
            std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
            let store = crate::store::Store::open(path.clone()).unwrap();
            assert_eq!(store.data.settings.appearance, Appearance::Light);
            assert_eq!(store.data.settings, expected);
        }
    }

    #[test]
    fn schedule_overrides_appearance_and_system_preference() {
        use chrono::Timelike;
        let now = chrono::Local::now();
        let minutes = now.hour() * 60 + now.minute();
        let day = (minutes + 1439) % 1440;
        let night = (minutes + 1) % 1440;
        for appearance in [Appearance::Light, Appearance::Dark, Appearance::System] {
            for system_dark in [false, true] {
                let mut settings = Settings { appearance, ..Settings::default() };
                assert_eq!(settings.is_dark(system_dark), appearance.is_dark(system_dark));
                settings.theme_schedule = true;
                settings.theme_day_start = format!("{:02}:{:02}", day / 60, day % 60);
                settings.theme_night_start = format!("{:02}:{:02}", night / 60, night % 60);
                assert!(!settings.is_dark(system_dark));
                std::mem::swap(&mut settings.theme_day_start, &mut settings.theme_night_start);
                assert!(settings.is_dark(system_dark));
            }
        }
    }

    #[test]
    fn schedule_uses_inclusive_day_and_exclusive_night_with_midnight_wrap() {
        for (day, night, minutes, dark) in [
            ("07:00", "19:00", 419, true), ("07:00", "19:00", 420, false),
            ("07:00", "19:00", 1139, false), ("07:00", "19:00", 1140, true),
            ("20:00", "06:00", 0, false), ("20:00", "06:00", 359, false),
            ("20:00", "06:00", 360, true), ("20:00", "06:00", 1199, true),
            ("20:00", "06:00", 1200, false), ("bad", "bad", 420, false),
        ] {
            assert_eq!(schedule_is_dark(day, night, minutes), dark);
        }
    }

    #[test]
    fn default_scheme_follows_app_theme() {
        assert_eq!(ColorScheme::Default.resolve(true), ColorScheme::Default);
        assert_eq!(ColorScheme::Default.resolve(false), ColorScheme::Light);
        assert_eq!(ColorScheme::Monokai.resolve(false), ColorScheme::Monokai);
        assert_eq!(ColorScheme::Light.resolve(true), ColorScheme::Light);
    }

    #[test]
    fn schemes_have_distinct_colors_and_map_defaults() {
        assert_eq!(ColorScheme::Default.palette(), crate::settings::ANSI_COLORS);
        for scheme in ColorScheme::ALL {
            assert_eq!(scheme.palette().len(), 16);
            assert_eq!(
                scheme.map_color(
                    crate::settings::DEFAULT_FG,
                    crate::settings::ColorSource::DefaultForeground
                ),
                scheme.foreground()
            );
            assert_eq!(
                scheme.map_color(
                    crate::settings::DEFAULT_BG,
                    crate::settings::ColorSource::DefaultBackground
                ),
                scheme.background()
            );
            assert_eq!(
                scheme.map_color(
                    crate::settings::ANSI_COLORS[1],
                    crate::settings::ColorSource::Ansi(1)
                ),
                scheme.palette()[1]
            );
            assert_eq!(
                scheme.map_color(0x123456, crate::settings::ColorSource::Explicit),
                0x123456
            );
            assert_eq!(
                scheme.map_color(
                    crate::settings::DEFAULT_FG,
                    crate::settings::ColorSource::Explicit
                ),
                crate::settings::DEFAULT_FG
            );
        }
        assert_ne!(
            ColorScheme::Light.background(),
            ColorScheme::Monokai.background()
        );
        assert_ne!(
            ColorScheme::SolarizedDark.foreground(),
            ColorScheme::Light.foreground()
        );
        assert_eq!(ColorScheme::from_index(99), ColorScheme::Light);
        assert_eq!(ColorScheme::from_index(-1), ColorScheme::Default);
        for (index, scheme) in ColorScheme::ALL.iter().enumerate() {
            assert_eq!(scheme.index(), index as i32);
            assert_eq!(ColorScheme::from_key(scheme.key()), *scheme);
            assert!(!scheme.label().is_empty());
        }
        assert_eq!(
            ColorScheme::from_key("solarized_dark"),
            ColorScheme::SolarizedDark
        );
        assert_eq!(BellMode::from_index(0), BellMode::None);
        assert_eq!(BellMode::from_index(7), BellMode::Visual);
        assert_eq!(BellMode::None.index(), 0);
        assert_eq!(BellMode::Visual.index(), 1);
    }

    #[test]
    fn persist_writes_and_rolls_back_cleanly() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(temp.path().join("data.json")).unwrap();
        let settings = Settings {
            font_size: 20,
            scrollback_lines: 10_000,
            ..Settings::default()
        };
        let saved = persist(&mut store, settings.clone()).unwrap();
        assert_eq!(saved, settings);
        let reopened = crate::store::Store::open(store.path.clone()).unwrap();
        assert_eq!(reopened.data.settings, settings);
        assert!(hidden_name(".bashrc"));
        assert!(!hidden_name("readme.txt"));
    }
}
