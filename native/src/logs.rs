//! Local connection history, kept in its own file next to the configuration.
//!
//! The history uses JSON Lines on disk and is capped to the most recent
//! [`MAX_ENTRIES`] sessions. Every mutation rewrites the file atomically so a
//! crash never leaves a partially written log behind. Only metadata is stored:
//! credentials, command lines and terminal output are never written here.
use anyhow::{Result, ensure};
use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Maximum number of sessions kept in the history file.
pub const MAX_ENTRIES: usize = 1000;
/// File name used inside the data directory.
pub const FILE_NAME: &str = "opentermius-logs.jsonl";

pub const CONNECTING: &str = "connecting";
pub const CONNECTED: &str = "connected";
pub const CLOSED: &str = "closed";
pub const FAILED: &str = "failed";

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LABEL: usize = 128;
const MAX_ADDRESS: usize = 300;
const MAX_ERROR: usize = 500;
const MAX_ID: usize = 64;

/// One connection attempt in the local history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub host_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl LogEntry {
    /// Whether a decoded line is usable. Corrupt or unknown lines are skipped.
    fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= MAX_ID
            && self.host_id.len() <= MAX_ID
            && !self.label.is_empty()
            && self.label.len() <= MAX_LABEL
            && !self.address.is_empty()
            && self.address.len() <= MAX_ADDRESS
            && valid_kind(&self.kind)
            && valid_status(&self.status)
            && parse_time(&self.started_at).is_some()
            && self
                .ended_at
                .as_deref()
                .is_none_or(|v| parse_time(v).is_some())
            && self.error.as_deref().is_none_or(|v| v.len() <= MAX_ERROR)
            && !has_control_chars(&self.label)
            && !has_control_chars(&self.address)
            && self.error.as_deref().is_none_or(|v| !has_control_chars(v))
    }
}

/// Connection history stored as JSON Lines next to the main configuration.
pub struct LogBook {
    path: PathBuf,
    entries: Vec<LogEntry>,
}

impl LogBook {
    /// Opens the history in `dir` (creating the directory if needed).
    ///
    /// Malformed lines are ignored rather than failing the whole history. If
    /// the file is longer than the cap, only the newest entries are kept and
    /// the file is compacted.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let mut entries = Vec::new();
        let mut skipped = false;
        if path.exists() {
            ensure!(fs::metadata(&path)?.len() < MAX_FILE_BYTES, "{}", crate::i18n::t("Connection history file is too large"));
            let bytes = fs::read(&path)?;
            for line in String::from_utf8_lossy(&bytes).lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(line) {
                    Ok(entry) if entry.valid() => entries.push(entry),
                    _ => skipped = true,
                }
            }
        }
        let truncated = entries.len() > MAX_ENTRIES;
        if truncated {
            entries.drain(..entries.len() - MAX_ENTRIES);
        }
        let book = Self { path, entries };
        if truncated || skipped {
            book.persist()?;
        }
        Ok(book)
    }

    /// Path of the JSON Lines file backing this history.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All entries in append order, oldest first.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Starts a history entry and returns its generated id.
    pub fn start(
        &mut self,
        host_id: &str,
        label: &str,
        address: &str,
        kind: &str,
    ) -> Result<String> {
        ensure!(valid_kind(kind), "{}", crate::i18n::t("Unknown session type"));
        let id = Uuid::new_v4().to_string();
        let address = clean(address, MAX_ADDRESS);
        let label = match clean(label, MAX_LABEL) {
            value if value.is_empty() => clean(&address, MAX_LABEL),
            value => value,
        };
        ensure!(!address.is_empty(), "{}", crate::i18n::t("Session address cannot be empty"));
        let entry = LogEntry {
            id: id.clone(),
            host_id: clean(host_id, MAX_ID),
            label,
            address,
            kind: kind.into(),
            started_at: now(),
            ended_at: None,
            status: CONNECTING.into(),
            error: None,
        };
        let previous = self.entries.clone();
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        if let Err(error) = self.persist() {
            self.entries = previous;
            return Err(error);
        }
        Ok(id)
    }

    /// Updates the status of an entry. Unknown ids are ignored so a late event
    /// from a cleared history stays harmless.
    pub fn update(&mut self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        ensure!(valid_status(status), "{}", crate::i18n::t("Unknown session status"));
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(());
        };
        let previous = self.entries.clone();
        let entry = &mut self.entries[index];
        entry.status = status.into();
        entry.error = if status == FAILED {
            error
                .map(|value| clean(value, MAX_ERROR))
                .filter(|v| !v.is_empty())
        } else {
            None
        };
        if is_final(status) {
            if entry.ended_at.is_none() {
                entry.ended_at = Some(now());
            }
        } else {
            entry.ended_at = None;
        }
        if let Err(error) = self.persist() {
            self.entries = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Marks an entry as finished. Failed entries keep their failure status.
    pub fn finish(&mut self, id: &str) -> Result<()> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(());
        };
        if self.entries[index].status == FAILED && self.entries[index].ended_at.is_some() {
            return Ok(());
        }
        let previous = self.entries.clone();
        let entry = &mut self.entries[index];
        if entry.status != FAILED {
            entry.status = CLOSED.into();
        }
        if entry.ended_at.is_none() {
            entry.ended_at = Some(now());
        }
        if let Err(error) = self.persist() {
            self.entries = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Newest entries first, capped to `limit`.
    pub fn recent(&self, limit: usize) -> Vec<LogEntry> {
        self.entries.iter().rev().take(limit).cloned().collect()
    }

    /// Removes every entry and truncates the history file.
    pub fn clear(&mut self) -> Result<()> {
        if self.entries.is_empty() && !self.path.exists() {
            return Ok(());
        }
        let previous = std::mem::take(&mut self.entries);
        if let Err(error) = self.persist() {
            self.entries = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Rewrites the whole history atomically (temporary file + rename).
    fn persist(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{}", crate::i18n::t("Invalid data directory")))?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!("opentermius-logs-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            for entry in &self.entries {
                serde_json::to_writer(&mut file, entry)?;
                file.write_all(b"\n")?;
            }
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

/// Parses an RFC 3339 timestamp produced by [`start`](LogBook::start).
pub fn parse_time(value: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Local))
}

fn now() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn valid_kind(value: &str) -> bool {
    matches!(value, "ssh" | "local")
}

fn valid_status(value: &str) -> bool {
    matches!(value, CONNECTING | CONNECTED | CLOSED | FAILED)
}

fn is_final(status: &str) -> bool {
    matches!(status, CLOSED | FAILED)
}

fn has_control_chars(value: &str) -> bool {
    value.chars().any(char::is_control)
}

/// Trims control characters and caps a user supplied string so a corrupt or
/// hostile value can never bloat the history file or break the UI.
fn clean(value: &str, max: usize) -> String {
    let mut bytes = 0;
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take_while(|ch| {
            bytes += ch.len_utf8();
            bytes <= max
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Whole seconds between the start of an entry and its end (or `now` while it
/// is still running).
pub fn elapsed_seconds(
    started_at: &str,
    ended_at: Option<&str>,
    now: DateTime<Local>,
) -> Option<i64> {
    let start = parse_time(started_at)?;
    let end = match ended_at {
        Some(value) => parse_time(value)?,
        None => now,
    };
    Some((end - start).num_seconds().max(0))
}

/// Compact human readable duration used by the history table.
pub fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        return crate::tf!("{seconds}s", "seconds" => format!("{}", seconds));
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        let rest = seconds % 60;
        return if rest == 0 {
            crate::tf!("{minutes}m", "minutes" => minutes.to_string())
        } else {
            crate::tf!("{minutes}m {rest}s", "minutes" => format!("{}", minutes), "rest" => format!("{}", rest))
        };
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    if hours < 24 {
        return if rest == 0 {
            crate::tf!("{hours}h", "hours" => hours.to_string())
        } else {
            crate::tf!("{hours}h {rest}m", "hours" => format!("{}", hours), "rest" => format!("{}", rest))
        };
    }
    let days = hours / 24;
    let rest = hours % 24;
    if rest == 0 {
        crate::tf!("{days}d", "days" => days.to_string())
    } else {
        crate::tf!("{days}d {rest}h", "days" => format!("{}", days), "rest" => format!("{}", rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> (tempfile::TempDir, LogBook) {
        let temp = tempfile::tempdir().unwrap();
        let book = LogBook::open(temp.path()).unwrap();
        (temp, book)
    }

    fn sample(id: &str, started_at: &str, status: &str) -> String {
        format!(
            r#"{{"id":"{id}","hostId":"host-1","label":"Dev box","address":"tester@example.invalid:22","kind":"ssh","startedAt":"{started_at}","endedAt":null,"status":"{status}","error":null}}"#
        )
    }

    #[test]
    fn append_update_and_finish_are_persisted() {
        let (temp, mut log) = book();
        let id = log
            .start("host-1", "Dev box", "tester@example.invalid:22", "ssh")
            .unwrap();
        assert_eq!(log.recent(10)[0].status, CONNECTING);
        log.update(&id, CONNECTED, None).unwrap();
        assert_eq!(log.recent(10)[0].status, CONNECTED);
        assert!(log.recent(10)[0].ended_at.is_none());
        log.finish(&id).unwrap();
        let entry = &log.recent(10)[0];
        assert_eq!(entry.status, CLOSED);
        assert!(entry.ended_at.is_some());

        let reopened = LogBook::open(temp.path()).unwrap();
        assert_eq!(reopened.entries().len(), 1);
        assert_eq!(reopened.entries()[0].id, id);
        assert_eq!(reopened.entries()[0].status, CLOSED);
    }

    #[test]
    fn failures_keep_their_message_and_finish_is_idempotent() {
        let (_temp, mut log) = book();
        let id = log
            .start("host-1", "Dev box", "tester@example.invalid:22", "ssh")
            .unwrap();
        log.update(&id, FAILED, Some("connection refused\n"))
            .unwrap();
        let failed = &log.recent(1)[0];
        assert_eq!(failed.status, FAILED);
        assert_eq!(failed.error.as_deref(), Some("connection refused"));
        let finished_at = failed.ended_at.clone();
        log.finish(&id).unwrap();
        assert_eq!(log.recent(1)[0].status, FAILED);
        assert_eq!(log.recent(1)[0].ended_at, finished_at);
        assert!(log.update("missing", CONNECTED, None).is_ok());
        assert!(log.update(&id, "bogus", None).is_err());
    }

    #[test]
    fn ordering_is_newest_first() {
        let (_temp, mut log) = book();
        let first = log
            .start("a", "First", "a@example.invalid:22", "ssh")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = log
            .start("b", "Second", "b@example.invalid:22", "ssh")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let third = log.start("", "Local Terminal", "Local", "local").unwrap();
        let recent = log.recent(10);
        assert_eq!(
            recent.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec![third.as_str(), second.as_str(), first.as_str()]
        );
        assert_eq!(log.recent(2).len(), 2);
        assert!(log.recent(0).is_empty());
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        let content = [
            sample("valid-1", "2026-01-01T10:00:00+00:00", CONNECTED),
            "not json".to_string(),
            r#"{"id":"missing-fields"}"#.to_string(),
            sample("valid-2", "2026-01-01T10:05:00+00:00", CLOSED),
            r#"{"id":"bad-kind","hostId":"","label":"x","address":"x","kind":"telnet","startedAt":"2026-01-01T10:00:00+00:00","status":"connected"}"#.to_string(),
        ]
        .join("\n");
        fs::write(&path, content).unwrap();
        let log = LogBook::open(temp.path()).unwrap();
        assert_eq!(log.entries().len(), 2);
        assert_eq!(log.entries()[0].id, "valid-1");
        assert_eq!(log.entries()[1].id, "valid-2");
    }

    #[test]
    fn file_is_capped_to_the_newest_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        let mut lines = Vec::new();
        for index in 0..MAX_ENTRIES + 5 {
            lines.push(sample(
                &format!("id-{index}"),
                "2026-01-01T10:00:00+00:00",
                CLOSED,
            ));
        }
        fs::write(&path, lines.join("\n")).unwrap();
        let mut log = LogBook::open(temp.path()).unwrap();
        assert_eq!(log.entries().len(), MAX_ENTRIES);
        assert_eq!(log.entries()[0].id, "id-5");
        // The compaction is persisted, not just applied in memory.
        let reopened = LogBook::open(temp.path()).unwrap();
        assert_eq!(reopened.entries().len(), MAX_ENTRIES);
        assert_eq!(reopened.entries()[0].id, "id-5");

        let newest = log
            .start("host-2", "New", "new@example.invalid:22", "ssh")
            .unwrap();
        assert_eq!(log.entries().len(), MAX_ENTRIES);
        assert_eq!(log.entries().last().unwrap().id, newest);
        assert_eq!(log.entries()[0].id, "id-6");
    }

    #[test]
    fn clear_truncates_the_file() {
        let (temp, mut log) = book();
        log.start("a", "First", "a@example.invalid:22", "ssh")
            .unwrap();
        log.start("", "Local", "Local", "local").unwrap();
        log.clear().unwrap();
        assert!(log.entries().is_empty());
        assert!(LogBook::open(temp.path()).unwrap().entries().is_empty());
        log.clear().unwrap();
    }

    #[test]
    fn durations_are_formatted_compactly() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(42), "42s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(3725), "1h 2m");
        assert_eq!(format_duration(86_400), "1d");
        assert_eq!(format_duration(-3), "0s");
        let start = "2026-01-01T10:00:00Z";
        let end = "2026-01-01T10:01:05Z";
        let now = parse_time(end).unwrap();
        assert_eq!(elapsed_seconds(start, Some(end), now), Some(65));
        assert_eq!(elapsed_seconds(start, None, now), Some(65));
        assert_eq!(elapsed_seconds("bad", None, now), None);
    }

    #[test]
    fn entries_never_keep_control_characters() {
        let (_temp, mut log) = book();
        let id = log
            .start("host\u{1}", "Line\nBreak", "host\r:22", "ssh")
            .unwrap();
        let entry = &log.recent(1)[0];
        assert_eq!(entry.label, "LineBreak");
        assert_eq!(entry.address, "host:22");
        assert!(!entry.host_id.contains('\u{1}'));
        log.update(&id, FAILED, Some("bad\u{7}news")).unwrap();
        assert_eq!(log.recent(1)[0].error.as_deref(), Some("badnews"));
    }

    #[test]
    fn long_unicode_metadata_survives_reopening() {
        let (temp, mut log) = book();
        let text = "连接记录🦀".repeat(200);
        let id = log.start("host", &text, &text, "ssh").unwrap();
        log.update(&id, FAILED, Some(&text)).unwrap();
        let entry = log.recent(1).remove(0);
        assert!(entry.valid());
        assert_eq!(LogBook::open(temp.path()).unwrap().recent(1), vec![entry]);
        log.start("host", "", &text, "ssh").unwrap();
        assert!(log.recent(1)[0].valid());
        assert_eq!(LogBook::open(temp.path()).unwrap().entries().len(), 2);
    }

    #[test]
    fn failed_writes_roll_back_memory_and_clean_temporary_files() {
        let (temp, mut log) = book();
        let id = log
            .start("a", "First", "a@example.invalid:22", "ssh")
            .unwrap();
        let original = log.recent(10);
        // A directory at the destination reliably makes rename fail on Windows.
        fs::remove_file(log.path()).unwrap();
        fs::create_dir(log.path()).unwrap();
        assert!(log.start("b", "Second", "Local", "local").is_err());
        assert_eq!(log.recent(10), original);
        assert!(log.update(&id, FAILED, Some("failure")).is_err());
        assert_eq!(log.recent(10), original);
        assert!(log.finish(&id).is_err());
        assert_eq!(log.recent(10), original);
        assert!(log.clear().is_err());
        assert_eq!(log.recent(10), original);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn late_events_cannot_restore_cleared_history() {
        let (temp, mut log) = book();
        let id = log.start("a", "First", "Local", "local").unwrap();
        log.clear().unwrap();
        log.update(&id, CONNECTED, None).unwrap();
        log.finish(&id).unwrap();
        assert!(LogBook::open(temp.path()).unwrap().recent(10).is_empty());
    }
}
