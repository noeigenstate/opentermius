//! Offline translations. English source text is the stable message key.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "i18n_catalog.rs"]
mod catalog;
use catalog::MESSAGES;

static CHINESE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
    #[default]
    #[serde(rename = "system")]
    System,
}

impl Language {
    pub const fn index(self) -> i32 {
        match self {
            Self::English => 0,
            Self::Chinese => 1,
            Self::System => 2,
        }
    }
    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::English,
            1 => Self::Chinese,
            _ => Self::System,
        }
    }
    pub const fn resolve(self, system_chinese: bool) -> Self {
        match self {
            Self::System if system_chinese => Self::Chinese,
            Self::System => Self::English,
            other => other,
        }
    }
}

pub fn locale_is_chinese(locale: &str) -> bool {
    locale
        .split(['_', '-', '.', '@'])
        .next()
        .is_some_and(|base| base.eq_ignore_ascii_case("zh"))
}

pub fn system_chinese() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // LANGID's low ten bits are the primary language identifier (Chinese = 4).
        unsafe { GetUserDefaultUILanguage() & 0x03ff == 0x0004 }
    }
    #[cfg(not(windows))]
    {
        std::env::var("LANG").is_ok_and(|locale| locale_is_chinese(&locale))
    }
}

/// Select Rust messages before constructing UI, then select Slint after creation.
pub fn set_language(language: Language) -> &'static str {
    let chinese = language.resolve(system_chinese()) == Language::Chinese;
    CHINESE.store(chinese, Ordering::Relaxed);
    if chinese { "zh_CN" } else { "en" }
}

pub fn t(key: &'static str) -> &'static str {
    if CHINESE.load(Ordering::Relaxed) {
        MESSAGES
            .iter()
            .find(|(en, _)| *en == key)
            .map_or(key, |(_, zh)| *zh)
    } else {
        key
    }
}

/// Match app error categories independently of the language used by a worker.
pub fn contains(message: &str, key: &'static str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains(&key.to_ascii_lowercase())
        || MESSAGES
            .iter()
            .find(|(en, _)| *en == key)
            .is_some_and(|(_, zh)| lower.contains(zh))
}

fn interpolate(template: &str, values: &[(&str, String)]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    let mut position = 0;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|end| start + end) else {
            break;
        };
        let field = &rest[start + 1..end];
        let slot = if field.is_empty() || field.starts_with(':') {
            let slot = format!("{position}{field}");
            position += 1;
            slot
        } else {
            field.to_string()
        };
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == slot) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn format(key: &'static str, values: &[(&str, String)]) -> String {
    let values = values
        .iter()
        .map(|(name, value)| {
            (
                *name,
                if matches!(name.split(':').next(), Some("error" | "e")) {
                    translate(value)
                } else {
                    value.clone()
                },
            )
        })
        .collect::<Vec<_>>();
    interpolate(t(key), &values)
}

/// Values are formatted by Rust at the call site, retaining Debug/Display specs.
#[macro_export]
macro_rules! tf {
    ($key:literal $(, $slot:literal => $value:expr)* $(,)?) => {
        $crate::i18n::format($key, &[$(($slot, $value)),*])
    };
}

fn fields(template: &str) -> (Vec<&str>, Vec<String>) {
    let mut literals = Vec::new();
    let mut names = Vec::new();
    let mut rest = template;
    let mut position = 0;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}').map(|end| start + end) else {
            break;
        };
        literals.push(&rest[..start]);
        let field = &rest[start + 1..end];
        names.push(if field.is_empty() || field.starts_with(':') {
            let name = format!("{position}{field}");
            position += 1;
            name
        } else {
            field.to_string()
        });
        rest = &rest[end + 1..];
    }
    literals.push(rest);
    (literals, names)
}

/// Re-render existing app messages and persisted summaries after a language change.
/// Unknown text (including server output and user data) passes through verbatim.
pub fn translate(message: &str) -> String {
    translate_to(message, CHINESE.load(Ordering::Relaxed))
}

fn translate_to(message: &str, chinese: bool) -> String {
    translate_inner(message, chinese, 0)
}

fn translate_inner(message: &str, chinese: bool, depth: usize) -> String {
    if depth > 8 {
        return message.to_string();
    }
    // Prefer exact matches so short status words cannot consume longer messages.
    for &(en, zh) in MESSAGES {
        if message == en || message == zh {
            return if chinese { zh } else { en }.to_string();
        }
    }
    let mut prefix = None;
    for &(en, zh) in MESSAGES {
        let target = if chinese { zh } else { en };
        let fragment = |text: &'static str| {
            text.strip_prefix(", ")
                .or_else(|| text.strip_prefix("; "))
                .or_else(|| text.strip_prefix('，'))
                .or_else(|| text.strip_prefix('；'))
        };
        for (source, is_fragment) in [(en, false), (zh, false)].into_iter().chain(
            [fragment(en), fragment(zh)]
                .into_iter()
                .flatten()
                .map(|source| (source, true)),
        ) {
            let target = if is_fragment {
                fragment(target).unwrap_or(target)
            } else {
                target
            };
            let (literals, names) = fields(source);
            if names.is_empty() || !message.starts_with(literals[0]) {
                continue;
            }
            let mut rest = &message[literals[0].len()..];
            let mut values = Vec::new();
            let mut matched = true;
            for (index, name) in names.iter().enumerate() {
                let next = literals[index + 1];
                let end = if next.is_empty() && index + 1 == names.len() {
                    Some(rest.len())
                } else {
                    rest.find(next)
                };
                let Some(end) = end else {
                    matched = false;
                    break;
                };
                let value = &rest[..end];
                if ((is_fragment || fragment(source).is_some())
                    && name.bytes().all(|byte| byte.is_ascii_digit())
                    || matches!(
                        name.split(':').next(),
                        Some(
                            "days"
                                | "hours"
                                | "minutes"
                                | "rest"
                                | "seconds"
                                | "count"
                                | "files"
                                | "dirs"
                                | "bytes"
                                | "size"
                                | "lines"
                                | "n"
                                | "port"
                        )
                    ))
                    && (value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
                {
                    matched = false;
                    break;
                }
                values.push((
                    name.as_str(),
                    if matches!(name.split(':').next(), Some("error" | "e")) {
                        translate_inner(value, chinese, depth + 1)
                    } else {
                        value.to_string()
                    },
                ));
                rest = &rest[end + next.len()..];
            }
            if matched && rest.is_empty() {
                return interpolate(target, &values);
            }
            if matched
                && [", ", "，", "; ", "；"]
                    .iter()
                    .any(|separator| rest.starts_with(separator))
            {
                let consumed = message.len() - rest.len();
                if prefix
                    .as_ref()
                    .is_none_or(|(length, _, _)| consumed > *length)
                {
                    prefix = Some((consumed, interpolate(target, &values), rest));
                }
            }
        }
    }
    if let Some((_, translated, rest)) = prefix {
        return format!("{translated}{}", translate_inner(rest, chinese, depth + 1));
    }
    // Anyhow adds context with ': '; existing import summaries append clauses.
    // Translate those app-owned fragments while leaving unknown details intact.
    for separator in [": ", ", ", "，", "; ", "；"] {
        if let Some((head, tail)) = message.split_once(separator) {
            let first = translate_inner(head, chinese, depth + 1);
            let second = translate_inner(tail, chinese, depth + 1);
            if first != head || second != tail {
                let separator = if separator == "，" && !chinese {
                    ", "
                } else if separator == ", " && chinese {
                    "，"
                } else if separator == "；" && !chinese {
                    "; "
                } else if separator == "; " && chinese {
                    "；"
                } else {
                    separator
                };
                return format!("{first}{separator}{second}");
            }
        }
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_key_has_both_languages_and_matching_placeholders() {
        let mut keys = BTreeSet::new();
        for &(en, zh) in MESSAGES {
            assert!(keys.insert(en), "duplicate key: {en}");
            assert!(
                !en.is_empty() && !zh.is_empty(),
                "missing translation: {en}"
            );
            let mut a = fields(en).1;
            let mut b = fields(zh).1;
            a.sort();
            b.sort();
            assert_eq!(a, b, "placeholder mismatch: {en}");
        }
    }

    #[test]
    fn language_selection_and_serialization() {
        for (language, code) in [
            (Language::English, "en"),
            (Language::Chinese, "zh-CN"),
            (Language::System, "system"),
        ] {
            assert_eq!(serde_json::to_value(language).unwrap(), code);
            assert_eq!(
                serde_json::from_value::<Language>(serde_json::json!(code)).unwrap(),
                language
            );
            assert_eq!(Language::from_index(language.index()), language);
        }
        assert_eq!(Language::default(), Language::System);
        assert_eq!(Language::System.resolve(true), Language::Chinese);
        assert_eq!(Language::System.resolve(false), Language::English);
        assert_eq!(Language::English.resolve(true), Language::English);
        assert_eq!(Language::Chinese.resolve(false), Language::Chinese);
        for locale in ["zh_CN.UTF-8", "zh-Hans-CN", "zh_TW", "ZH"] {
            assert!(locale_is_chinese(locale));
        }
        for locale in ["en_US.UTF-8", "C", "", "zhang"] {
            assert!(!locale_is_chinese(locale));
        }
        let settings: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.language, Language::System);
    }

    #[test]
    fn rust_call_sites_are_present_in_the_catalog() {
        let keys = MESSAGES.iter().map(|(en, _)| *en).collect::<BTreeSet<_>>();
        let mut directories = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    directories.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs")
                    || path
                        .file_name()
                        .is_some_and(|name| name == "i18n.rs" || name == "i18n_catalog.rs")
                {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                let source = source.split("#[cfg(test)]").next().unwrap();
                for marker in ["i18n::t(", "tf!("] {
                    for tail in source.split(marker).skip(1) {
                        if !tail.trim_start().starts_with('"') {
                            continue;
                        }
                        let mut stream =
                            serde_json::Deserializer::from_str(tail).into_iter::<String>();
                        let key = stream.next().unwrap().unwrap();
                        assert!(
                            keys.contains(key.as_str()),
                            "missing Rust translation in {}: {key}",
                            path.display()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rendered_messages_refresh_in_both_directions_without_changing_user_text() {
        let outside_root = "Local path is outside the selected folder";
        assert_eq!(translate_to(outside_root, true), "本地路径不在所选文件夹内");
        assert_eq!(translate_to("本地路径不在所选文件夹内", false), outside_root);
        assert_eq!(translate_to("Connected", true), "已连接");
        assert_eq!(translate_to("已连接", false), "Connected");
        assert_eq!(translate_to("已导入 12 台主机", false), "Imported 12 hosts");
        assert_eq!(translate_to("Imported 12 hosts", true), "已导入 12 台主机");
        assert_eq!(
            translate_to("Uploaded report.txt, 123 bytes", true),
            "已上传 report.txt，123 字节"
        );
        assert_eq!(
            translate_to("Cannot save view settings: Host not found", true),
            "无法保存视图设置：主机不存在"
        );
        let xshell = "Imported 1 hosts, 2 duplicates, 3 skipped; passwords were excluded. Enter credentials on first connect; 4 hosts used public keys and now use password authentication. Select a private key in Host Details";
        assert_eq!(translate_to(xshell, true), "已导入 1 台主机，重复 2 台，跳过 3 项；密码未导入，首次连接时需输入凭据；其中 4 台原使用公钥，已暂设为密码认证，请在主机详情中选择私钥");
        assert_eq!(translate_to(&translate_to(xshell, true), false), xshell);
        assert_eq!(
            translate_to("user@server: arbitrary text", true),
            "user@server: arbitrary text"
        );
        assert_eq!(
            translate_to("arbitrary server text ending in d", true),
            "arbitrary server text ending in d"
        );
        assert_eq!(
            interpolate(
                "{error:#} / {}",
                &[("error:#", "details".into()), ("0", "2".into())]
            ),
            "details / 2"
        );
    }
}
