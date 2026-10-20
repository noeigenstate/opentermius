//! Reusable shell command snippets.
//!
//! A snippet is a named, multi-line command that can be sent to the active
//! terminal session. The data model, validation, search and terminal input
//! encoding live here so they can be unit-tested without a UI.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_NAME_CHARS: usize = 128;
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
pub const MAX_TAGS: usize = 8;
pub const MAX_TAG_CHARS: usize = 32;

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub script: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

impl Snippet {
    /// Trim and validate the user supplied fields, normalise tags and assign
    /// an id when the snippet is new.
    pub fn validate(&mut self) -> Result<()> {
        let name = self.name.trim();
        ensure!(!name.is_empty(), "名称不能为空");
        ensure!(
            name.chars().count() <= MAX_NAME_CHARS,
            "名称不能超过 {MAX_NAME_CHARS} 个字符"
        );
        ensure!(!name.chars().any(char::is_control), "名称包含无效字符");
        self.name = name.to_string();
        ensure!(!self.script.trim().is_empty(), "脚本不能为空");
        ensure!(
            self.script.len() <= MAX_SCRIPT_BYTES,
            "脚本不能超过 {} KB",
            MAX_SCRIPT_BYTES / 1024
        );
        self.tags = normalize_tags(std::mem::take(&mut self.tags))?;
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        }
        Ok(())
    }
}

/// Split a comma/semicolon separated tag field into individual tags.
/// Validation of the resulting tags happens in [`Snippet::validate`].
pub fn parse_tags(value: &str) -> Vec<String> {
    value
        .split([',', ';', '，', '；'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>> {
    let mut normalized: Vec<String> = Vec::new();
    for raw in tags {
        let tag = raw.trim();
        if tag.is_empty() {
            continue;
        }
        ensure!(!tag.chars().any(char::is_control), "标签包含无效字符");
        ensure!(
            tag.chars().count() <= MAX_TAG_CHARS,
            "单个标签不能超过 {MAX_TAG_CHARS} 个字符"
        );
        if !normalized
            .iter()
            .any(|existing| existing.to_lowercase() == tag.to_lowercase())
        {
            normalized.push(tag.to_string());
        }
    }
    ensure!(normalized.len() <= MAX_TAGS, "标签不能超过 {MAX_TAGS} 个");
    Ok(normalized)
}

fn sort(snippets: &mut [Snippet]) {
    snippets.sort_by(|a, b| {
        b.updated_at
            .unwrap_or_default()
            .cmp(&a.updated_at.unwrap_or_default())
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Insert a new snippet or replace an existing one. Returns the snippet id.
///
/// `now` is a Unix timestamp in milliseconds; taking it as an argument keeps
/// tests deterministic. The creation time of an existing snippet is kept.
pub fn save(snippets: &mut Vec<Snippet>, mut snippet: Snippet, now: i64) -> Result<String> {
    snippet.validate()?;
    let id = snippet.id.clone();
    match snippets
        .iter_mut()
        .find(|existing| existing.id == snippet.id)
    {
        Some(existing) => {
            snippet.created_at = existing.created_at.or(Some(now));
            snippet.updated_at = Some(now);
            *existing = snippet;
        }
        None => {
            snippet.created_at = Some(now);
            snippet.updated_at = Some(now);
            snippets.push(snippet);
        }
    }
    sort(snippets);
    Ok(id)
}

/// Remove a snippet by id. Returns whether a snippet was removed.
pub fn delete(snippets: &mut Vec<Snippet>, id: &str) -> bool {
    let before = snippets.len();
    snippets.retain(|snippet| snippet.id != id);
    snippets.len() != before
}

pub fn find<'a>(snippets: &'a [Snippet], id: &str) -> Option<&'a Snippet> {
    snippets.iter().find(|snippet| snippet.id == id)
}

/// Case-insensitive search over name, script and tags.
pub fn search<'a>(snippets: &'a [Snippet], query: &str) -> Vec<&'a Snippet> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return snippets.iter().collect();
    }
    snippets
        .iter()
        .filter(|snippet| {
            snippet.name.to_lowercase().contains(&query)
                || snippet.script.to_lowercase().contains(&query)
                || snippet
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query))
        })
        .collect()
}

/// First non-empty line of a script, suitable for a card preview.
pub fn first_line(script: &str) -> &str {
    script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// Human readable local timestamp for a card footer.
pub fn format_timestamp(value: Option<i64>) -> String {
    value
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Encode a snippet for a terminal session.
///
/// Without bracketed paste support every line ending becomes `\r`, because a
/// carriage return is what a terminal expects for Enter. With bracketed paste
/// the text is wrapped in `ESC[200~`/`ESC[201~` (mirroring `State::paste`) and
/// embedded ESC bytes are stripped so a snippet cannot inject terminal control
/// sequences. Trailing line endings are dropped and exactly one `\r` is
/// appended, so running a snippet executes it without submitting an extra
/// empty command.
pub fn to_terminal_input(script: &str, bracketed: bool) -> Vec<u8> {
    let content = if bracketed {
        script.replace('\x1b', "")
    } else {
        script
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r")
    };
    let content = content.trim_end_matches(['\r', '\n']);
    let mut input = Vec::with_capacity(content.len() + 16);
    if bracketed {
        input.extend_from_slice(BRACKETED_PASTE_START);
        input.extend_from_slice(content.as_bytes());
        input.extend_from_slice(BRACKETED_PASTE_END);
    } else {
        input.extend_from_slice(content.as_bytes());
    }
    input.push(b'\r');
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str, script: &str) -> Snippet {
        Snippet {
            name: name.into(),
            script: script.into(),
            ..Snippet::default()
        }
    }

    #[test]
    fn validation_requires_name_and_script() {
        let mut unnamed = snippet("   ", "echo hello");
        assert!(unnamed.validate().is_err());
        let mut empty = snippet("Deploy", " \n\t ");
        assert!(empty.validate().is_err());
    }

    #[test]
    fn validation_enforces_script_size_limit() {
        let mut too_long = snippet("Large", &"x".repeat(MAX_SCRIPT_BYTES + 1));
        assert!(too_long.validate().is_err());
        let mut limit = snippet("Limit", &"x".repeat(MAX_SCRIPT_BYTES));
        assert!(limit.validate().is_ok());
    }

    #[test]
    fn validation_counts_name_characters_and_script_bytes() {
        let mut value = snippet(&"界".repeat(MAX_NAME_CHARS), "echo 你好");
        assert!(value.validate().is_ok());
        value.name.push('界');
        assert!(value.validate().is_err());

        let mut oversized = snippet("Unicode", &"界".repeat(MAX_SCRIPT_BYTES / 3 + 1));
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn validation_rejects_control_characters_in_names_and_tags() {
        let mut value = snippet("Deploy\nproduction", "echo hello");
        assert!(value.validate().is_err());
        value.name = "Deploy".into();
        value.tags = vec!["prod\tweb".into()];
        assert!(value.validate().is_err());
    }

    #[test]
    fn validation_trims_name_assigns_id_and_normalises_tags() {
        let mut value = Snippet {
            name: "  Deploy  ".into(),
            script: "echo hi".into(),
            tags: vec![" prod ".into(), "".into(), "PROD".into(), "web".into()],
            ..Snippet::default()
        };
        value.validate().unwrap();
        assert_eq!(value.name, "Deploy");
        assert!(!value.id.is_empty());
        assert_eq!(value.tags, ["prod", "web"]);
    }

    #[test]
    fn validation_rejects_too_many_or_oversized_tags() {
        let mut too_many = snippet("Tags", "echo");
        too_many.tags = (0..=MAX_TAGS).map(|i| format!("tag{i}")).collect();
        assert!(too_many.validate().is_err());

        let mut too_long = snippet("Tags", "echo");
        too_long.tags = vec!["x".repeat(MAX_TAG_CHARS + 1)];
        assert!(too_long.validate().is_err());
    }

    #[test]
    fn save_inserts_updates_preserves_created_time_and_sorts() {
        let mut list = Vec::new();
        let first = save(&mut list, snippet("First", "echo one"), 100).unwrap();
        let second = save(&mut list, snippet("Second", "echo two"), 200).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, second);
        assert_eq!(list[0].created_at, Some(200));

        let mut edit = list.iter().find(|s| s.id == first).unwrap().clone();
        edit.name = "First edited".into();
        save(&mut list, edit, 300).unwrap();
        let updated = list.iter().find(|s| s.id == first).unwrap();
        assert_eq!(updated.name, "First edited");
        assert_eq!(updated.created_at, Some(100));
        assert_eq!(updated.updated_at, Some(300));
        assert_eq!(list[0].id, first);
    }

    #[test]
    fn save_rejects_invalid_input_without_mutating_the_list() {
        let mut list = Vec::new();
        save(&mut list, snippet("Keep", "echo"), 100).unwrap();
        let before = list.clone();
        assert!(save(&mut list, snippet("", "echo"), 200).is_err());
        assert_eq!(list, before);
    }

    #[test]
    fn delete_removes_only_the_matching_snippet() {
        let mut list = Vec::new();
        save(&mut list, snippet("Keep", "echo"), 100).unwrap();
        let removable = save(&mut list, snippet("Remove", "echo"), 200).unwrap();
        assert!(delete(&mut list, &removable));
        assert_eq!(list.len(), 1);
        assert!(!delete(&mut list, &removable));
    }

    #[test]
    fn search_matches_name_script_and_tags_case_insensitively() {
        let mut list = Vec::new();
        let mut deploy = snippet("Deploy staging", "kubectl apply -f app.yaml");
        deploy.tags = vec!["Production".into()];
        save(&mut list, deploy, 100).unwrap();
        save(&mut list, snippet("List logs", "journalctl -u nginx"), 200).unwrap();

        assert_eq!(search(&list, "").len(), 2);
        assert_eq!(search(&list, "  DEPLOY ").len(), 1);
        assert_eq!(search(&list, "kubectl").len(), 1);
        assert_eq!(search(&list, "production").len(), 1);
        assert!(search(&list, "missing").is_empty());
    }

    #[test]
    fn newline_conversion_produces_exactly_one_trailing_cr() {
        assert_eq!(
            to_terminal_input("echo one\necho two", false),
            b"echo one\recho two\r"
        );
        assert_eq!(
            to_terminal_input("echo one\r\necho two\r\n", false),
            b"echo one\recho two\r"
        );
        assert_eq!(to_terminal_input("echo one\n\n", false), b"echo one\r");
        assert_eq!(to_terminal_input("", false), b"\r");
    }

    #[test]
    fn bracketed_input_wraps_content_and_strips_escape_bytes() {
        let input = to_terminal_input("printf \x1b[31mred\nsecond", true);
        assert_eq!(input, b"\x1b[200~printf [31mred\nsecond\x1b[201~\r");
        assert_eq!(input.iter().filter(|byte| **byte == 0x1b).count(), 2);
    }

    #[test]
    fn bracketed_input_preserves_internal_line_endings_and_submits_after_wrapper() {
        for script in ["echo one\necho two\n", "echo one\r\necho two\r\n"] {
            let mut expected = BRACKETED_PASTE_START.to_vec();
            expected.extend_from_slice(script.trim_end_matches(['\r', '\n']).as_bytes());
            expected.extend_from_slice(b"\x1b[201~\r");
            assert_eq!(to_terminal_input(script, true), expected);
        }
        assert_eq!(to_terminal_input("", true), b"\x1b[200~\x1b[201~\r");
    }

    #[test]
    fn embedded_paste_terminator_cannot_close_the_wrapper() {
        assert_eq!(
            to_terminal_input("echo safe\x1b[201~\r\necho 你好", true),
            "\x1b[200~echo safe[201~\r\necho 你好\x1b[201~\r".as_bytes()
        );
        assert_eq!(
            to_terminal_input("echo 你好\recho again\r", false),
            "echo 你好\recho again\r".as_bytes()
        );
    }

    #[test]
    fn preview_uses_first_non_empty_line() {
        assert_eq!(first_line("\n  \n echo hello \nworld"), "echo hello");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn parse_tags_accepts_commas_and_semicolons() {
        assert_eq!(
            parse_tags(" prod, web; ops，deploy；"),
            ["prod", "web", "ops", "deploy"]
        );
    }

    #[test]
    fn json_round_trip_uses_camel_case_and_defaults() {
        let value = snippet("Deploy", "echo hi");
        let json = serde_json::to_string(&value).unwrap();
        assert!(json.contains("\"createdAt\""));
        assert!(json.contains("\"updatedAt\""));
        let parsed: Snippet = serde_json::from_str(r#"{"name":"x","script":"y"}"#).unwrap();
        assert_eq!(parsed.name, "x");
        assert!(parsed.tags.is_empty());
        assert_eq!(parsed.created_at, None);
    }
}
