//! User preferences persisted with the local configuration.
//!
//! Every value is clamped to the range offered by the settings page. The module
//! also owns the terminal color schemes so rendering can be changed without
//! touching the SSH/SFTP transport.
use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
            Self::Default => crate::terminal::ANSI_COLORS,
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
    pub fn map_color(self, value: u32, source: crate::terminal::ColorSource) -> u32 {
        use crate::terminal::ColorSource;
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
    pub gpu_rendering: bool,
    pub keepalive_seconds: u32,
    pub connection_timeout_seconds: u32,
    pub confirm_overwrite: bool,
    pub show_hidden_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: crate::i18n::Language::default(),
            font_size: DEFAULT_FONT_SIZE,
            scrollback_lines: crate::terminal::DEFAULT_SCROLLBACK,
            cursor_blink: true,
            copy_on_select: false,
            confirm_multiline_paste: true,
            bell: BellMode::Visual,
            color_scheme: ColorScheme::Default,
            appearance: Appearance::default(),
            gpu_rendering: false,
            keepalive_seconds: KEEPALIVE_DEFAULT,
            connection_timeout_seconds: CONNECTION_TIMEOUT_DEFAULT,
            confirm_overwrite: true,
            show_hidden_files: true,
        }
    }
}

impl Settings {
    /// Clamp persisted or user-entered values to the ranges exposed by the UI.
    pub fn sanitize(&mut self) {
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
            crate::terminal::DEFAULT_SCROLLBACK
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
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
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
        assert_eq!(ColorScheme::Default.palette(), crate::terminal::ANSI_COLORS);
        for scheme in ColorScheme::ALL {
            assert_eq!(scheme.palette().len(), 16);
            assert_eq!(
                scheme.map_color(
                    crate::terminal::DEFAULT_FG,
                    crate::terminal::ColorSource::DefaultForeground
                ),
                scheme.foreground()
            );
            assert_eq!(
                scheme.map_color(
                    crate::terminal::DEFAULT_BG,
                    crate::terminal::ColorSource::DefaultBackground
                ),
                scheme.background()
            );
            assert_eq!(
                scheme.map_color(
                    crate::terminal::ANSI_COLORS[1],
                    crate::terminal::ColorSource::Ansi(1)
                ),
                scheme.palette()[1]
            );
            assert_eq!(
                scheme.map_color(0x123456, crate::terminal::ColorSource::Explicit),
                0x123456
            );
            assert_eq!(
                scheme.map_color(
                    crate::terminal::DEFAULT_FG,
                    crate::terminal::ColorSource::Explicit
                ),
                crate::terminal::DEFAULT_FG
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
