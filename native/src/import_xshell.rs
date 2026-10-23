//! Local Xshell session import. Vendor credentials and key-store names are never imported.
use crate::store::{Profile, Store};
use anyhow::{Result, ensure};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_SESSION_BYTES: u64 = 5 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct HostDraft {
    pub profile: Profile,
    /// Xshell keys cannot be resolved to a private-key file automatically.
    pub needs_key: bool,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub hosts: Vec<HostDraft>,
    pub skipped: Vec<(PathBuf, String)>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub needs_key: usize,
}

impl ImportSummary {
    pub fn message(&self) -> String {
        let mut message = crate::tf!("Imported {} hosts, {} duplicates, {} skipped; passwords were excluded. Enter credentials on first connect", "0" => format!("{}", self.imported), "1" => format!("{}", self.duplicates), "2" => format!("{}", self.skipped));
        if self.needs_key > 0 {
            message.push_str(&crate::tf!("; {} hosts used public keys and now use password authentication. Select a private key in Host Details", "0" => format!("{}", self.needs_key)));
        }
        message
    }
}

/// Recursively reads only `.xsh` files, without following directory symlinks.
/// Errors contain fixed reasons rather than session contents or credentials.
pub fn scan(dir: impl AsRef<Path>) -> ScanResult {
    let mut result = ScanResult::default();
    let mut seen = 0;
    visit(dir.as_ref(), dir.as_ref(), &mut result, &mut seen, 0);
    result
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn visit(root: &Path, dir: &Path, result: &mut ScanResult, seen: &mut usize, depth: usize) {
    if depth >= MAX_DEPTH {
        result.skipped.push((dir.into(), crate::i18n::t("Session directory nesting is too deep").into()));
        return;
    }
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
        _ => {
            result.skipped.push((dir.into(), crate::i18n::t("Session directory is invalid or is a link").into()));
            return;
        }
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            result.skipped.push((dir.into(), crate::i18n::t("Cannot read session directory").into()));
            return;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        if *seen >= MAX_ENTRIES {
            result.skipped.push((dir.into(), crate::i18n::t("Too many session directory entries").into()));
            break;
        }
        *seen += 1;
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => result.skipped.push((dir.into(), crate::i18n::t("Cannot read directory entry").into())),
        }
    }
    paths.sort();
    for path in paths {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                result.skipped.push((path, crate::i18n::t("Cannot read session file metadata").into()));
                continue;
            }
        };
        // Windows junctions are also reparse points; do not traverse them.
        if is_link(&metadata) {
            result.skipped.push((path, crate::i18n::t("Linked directories and files cannot be imported").into()));
        } else if metadata.is_dir() {
            visit(root, &path, result, seen, depth + 1);
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xsh"))
        {
            match parse_file(root, &path) {
                Ok(host) => result.hosts.push(host),
                Err(error) => result.skipped.push((path, error.to_string())),
            }
        }
    }
}

fn parse_file(root: &Path, path: &Path) -> Result<HostDraft> {
    let mut bytes = Zeroizing::new(Vec::new());
    fs::File::open(path)
        .and_then(|file| file.take(MAX_SESSION_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Cannot read session file")))?;
    ensure!(bytes.len() as u64 <= MAX_SESSION_BYTES, "{}", crate::i18n::t("Session file is too large"));
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        ensure!(bytes.len() % 2 == 0, "{}", crate::i18n::t("Invalid UTF-16 session encoding"));
        let words = Zeroizing::new(
            bytes[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        );
        Zeroizing::new(
            String::from_utf16(&words).map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Invalid UTF-16 session encoding")))?,
        )
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        Zeroizing::new(
            std::str::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Invalid session encoding")))?
                .to_owned(),
        )
    };
    let mut section = String::new();
    let (mut host, mut port, mut protocol, mut username) = ("", "", "", "");
    let (mut method, mut use_key, mut user_key) = ("", "", "");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') {
            ensure!(line.ends_with(']'), "{}", crate::i18n::t("Invalid session INI section"));
            section = line[1..line.len() - 1].trim().to_ascii_uppercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            // Ignore unrelated vendor sections, but reject broken connection data.
            ensure!(!matches!(section.as_str(), "CONNECTION" | "CONNECTION:AUTHENTICATION"), "{}", crate::i18n::t("Invalid session INI field"));
            continue;
        };
        let value = value.trim();
        match (section.as_str(), key.trim().to_ascii_uppercase().as_str()) {
            ("CONNECTION", "HOST") => host = value,
            ("CONNECTION", "PORT") => port = value,
            ("CONNECTION", "PROTOCOL") => protocol = value,
            ("CONNECTION:AUTHENTICATION", "USERNAME") => username = value,
            ("CONNECTION:AUTHENTICATION", "METHOD") => method = value,
            ("CONNECTION:AUTHENTICATION", "USEKEY") => use_key = value,
            ("CONNECTION:AUTHENTICATION", "USERKEY") => user_key = value,
            _ => {} // Password and Passphrase are deliberately never copied.
        }
    }
    ensure!(protocol.eq_ignore_ascii_case("SSH") || protocol.eq_ignore_ascii_case("SFTP"), "{}", crate::i18n::t("Only SSH and SFTP sessions are supported"));
    ensure!(matches!(method, "" | "0" | "1" | "2"), "{}", crate::i18n::t("Unsupported authentication method (such as GSSAPI)"));
    let needs_key = method == "1"
        || use_key == "1"
        || use_key.eq_ignore_ascii_case("true")
        || (method.is_empty() && !user_key.is_empty());
    let port = if port.is_empty() {
        22
    } else {
        port.parse::<u16>()
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Invalid session port")))?
    };
    let group = path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .map(|relative| {
            relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();
    let mut profile = Profile {
        id: String::new(),
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        host: host.into(),
        port,
        username: username.into(),
        auth_type: "password".into(),
        private_key_path: String::new(),
        key_id: String::new(),
        group,
        encrypted_secret: None,
        tags: Vec::new(),
        last_connected: None,
        favorite: false,
        created_at: None,
    };
    profile.validate()?;
    Ok(HostDraft { profile, needs_key })
}

/// Validates, deduplicates and persists a batch once, restoring memory on failure.
pub fn merge(store: &mut Store, scanned: ScanResult) -> Result<ImportSummary> {
    let mut summary = ImportSummary {
        skipped: scanned.skipped.len(),
        ..Default::default()
    };
    let identity = |profile: &Profile| {
        (
            profile.host.to_ascii_lowercase(),
            profile.port,
            profile.username.clone(),
        )
    };
    let mut seen: HashSet<_> = store.data.hosts.iter().map(identity).collect();
    let mut added = Vec::new();
    for draft in scanned.hosts {
        let mut profile = draft.profile;
        profile.id.clear();
        profile.encrypted_secret = None;
        if profile.validate().is_err() {
            summary.skipped += 1;
        } else if !seen.insert(identity(&profile)) {
            summary.duplicates += 1;
        } else {
            summary.needs_key += usize::from(draft.needs_key);
            profile.created_at = Some(crate::hosts::now_rfc3339());
            added.push(profile);
        }
    }
    summary.imported = added.len();
    if !added.is_empty() {
        let previous_len = store.data.hosts.len();
        store.data.hosts.extend(added);
        if let Err(error) = store.persist() {
            store.data.hosts.truncate(previous_len);
            return Err(error);
        }
    }
    Ok(summary)
}

/// Finds an existing Sessions directory; never reads the sessions themselves.
#[cfg(windows)]
pub fn default_sessions_dir() -> Option<PathBuf> {
    for version in [8, 7, 6, 5] {
        if let Some(path) = registry_user_data(version) {
            let sessions = path.join("Xshell").join("Sessions");
            if sessions.is_dir() {
                return Some(sessions);
            }
        }
    }
    let home = PathBuf::from(std::env::var_os("USERPROFILE")?);
    for version in [8, 7, 6, 5] {
        let sessions = home
            .join("Documents")
            .join("NetSarang Computer")
            .join(version.to_string())
            .join("Xshell")
            .join("Sessions");
        if sessions.is_dir() {
            return Some(sessions);
        }
    }
    let sessions = home
        .join("Documents")
        .join("NetSarang")
        .join("Xshell")
        .join("Sessions");
    sessions.is_dir().then_some(sessions)
}

#[cfg(not(windows))]
pub fn default_sessions_dir() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn registry_user_data(version: u8) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
        RegGetValueW,
    };
    let key: Vec<u16> = format!("Software\\NetSarang\\Common\\{version}\\UserData\0")
        .encode_utf16()
        .collect();
    let name: Vec<u16> = "UserDataPath\0".encode_utf16().collect();
    let mut size = 0;
    let mut value_type = 0;
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    // Read either string type, then expand REG_EXPAND_SZ explicitly.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        ) != 0
            || size < 2
            || size > 65536
        {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut size,
        ) != 0
        {
            return None;
        }
        if value_type == REG_EXPAND_SZ {
            let length = ExpandEnvironmentStringsW(buffer.as_ptr(), std::ptr::null_mut(), 0);
            if length == 0 || length > 32768 {
                return None;
            }
            let mut expanded = vec![0u16; length as usize];
            let written = ExpandEnvironmentStringsW(buffer.as_ptr(), expanded.as_mut_ptr(), length);
            if written == 0 || written > length {
                return None;
            }
            buffer = expanded;
        }
        let end = buffer.iter().position(|&word| word == 0)?;
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end])))
    }
}
