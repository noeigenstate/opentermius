//! Update check against the project's GitHub Releases. The only network requests are the
//! release metadata, `checksums.json` and the installer; nothing about the user is sent.
//! An installer is used only when its size and SHA-256 match `checksums.json`.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/noeigenstate/opentermius/releases/latest";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_INSTALLER_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    pub installer_name: String,
    pub installer_url: String,
    pub checksums_url: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Checksum {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}

/// `1.2.3`, `v1.2.3` or `1.2.3-beta` → (1, 2, 3). Pre-release suffixes are ignored.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(version)
}

pub fn is_newer(current: &str, candidate: &str) -> bool {
    matches!((parse_version(current), parse_version(candidate)), (Some(a), Some(b)) if b > a)
}

/// Picks the Windows installer and `checksums.json` from a GitHub release document.
pub fn parse_release(json: &str) -> Result<Release> {
    let value: Value = serde_json::from_str(json).context(crate::i18n::t("Invalid release information"))?;
    let tag = value.get("tag_name").and_then(Value::as_str).unwrap_or_default();
    let version = parse_version(tag).map(|(a, b, c)| format!("{a}.{b}.{c}")).context(crate::i18n::t("Invalid release information"))?;
    let assets = value.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let asset = |pick: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            let url = asset.get("browser_download_url")?.as_str()?;
            (pick(name) && url.starts_with("https://")).then(|| (name.to_string(), url.to_string()))
        })
    };
    let (installer_name, installer_url) = asset(&|name| name.ends_with("-windows-x64-setup.exe"))
        .context(crate::i18n::t("The release has no Windows installer"))?;
    let (_, checksums_url) = asset(&|name| name == "checksums.json").context(crate::i18n::t("The release has no checksums"))?;
    Ok(Release {
        version,
        page_url: value.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        installer_name,
        installer_url,
        checksums_url,
    })
}

pub fn parse_checksum(json: &str) -> Result<Checksum> {
    let value: Value = serde_json::from_str(json).context(crate::i18n::t("Invalid checksums"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let sha256 = text("sha256").unwrap_or_default().to_ascii_lowercase();
    ensure!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()), "{}", crate::i18n::t("Invalid checksums"));
    Ok(Checksum {
        file: text("file").context(crate::i18n::t("Invalid checksums"))?,
        bytes: value.get("bytes").and_then(Value::as_u64).context(crate::i18n::t("Invalid checksums"))?,
        sha256,
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The installer is accepted only when name, size and SHA-256 all match.
pub fn verify(installer: &[u8], name: &str, checksum: &Checksum) -> Result<()> {
    ensure!(checksum.file == name, "{}", crate::i18n::t("The checksums do not describe this installer"));
    ensure!(installer.len() as u64 == checksum.bytes, "{}", crate::i18n::t("The downloaded installer has the wrong size"));
    ensure!(sha256_hex(installer) == checksum.sha256, "{}", crate::i18n::t("The downloaded installer failed verification"));
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("OpenTermius/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json, application/octet-stream")
        .call()
        .context(crate::i18n::t("Cannot reach the update server"))?;
    let mut data = Vec::new();
    response.into_body().into_reader().take(limit + 1).read_to_end(&mut data).context(crate::i18n::t("Cannot reach the update server"))?;
    ensure!(data.len() as u64 <= limit, "{}", crate::i18n::t("The update download is too large"));
    Ok(data)
}

/// Returns the latest release when it is newer than `current`.
pub fn check(current: &str) -> Result<Option<Release>> {
    let data = get(&agent(), LATEST_RELEASE_API, MAX_METADATA_BYTES)?;
    let release = parse_release(&String::from_utf8_lossy(&data))?;
    Ok(is_newer(current, &release.version).then_some(release))
}

/// Downloads and verifies the installer into `dir`, returning its path.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf> {
    let agent = agent();
    let checksum = parse_checksum(&String::from_utf8_lossy(&get(&agent, &release.checksums_url, MAX_METADATA_BYTES)?))?;
    let installer = get(&agent, &release.installer_url, MAX_INSTALLER_BYTES)?;
    verify(&installer, &release.installer_name, &checksum)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(&release.installer_name);
    std::fs::write(&path, &installer)?;
    Ok(path)
}

/// Starts a copy of this program that waits for `pid` to exit, runs the installer
/// silently into `install_dir` and starts the new version. The installer refuses to run
/// while OpenTermius is open, so the running exe cannot do this itself.
#[cfg(windows)]
pub fn launch_installer(installer: &Path, install_dir: &Path, work_dir: &Path) -> Result<()> {
    let helper = work_dir.join("opentermius-updater.exe");
    std::fs::copy(std::env::current_exe()?, &helper)?;
    std::process::Command::new(&helper)
        .arg("--apply-update")
        .arg(installer)
        .arg(install_dir)
        .arg(std::process::id().to_string())
        .spawn()
        .context(crate::i18n::t("Cannot start the installer"))?;
    Ok(())
}

/// `--apply-update <installer> <install dir> <pid>`: runs in the helper copy.
#[cfg(windows)]
pub fn apply(args: &[String]) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
    let [installer, install_dir, pid] = args else { bail!("{}", crate::i18n::t("Invalid update arguments")) };
    let pid: u32 = pid.parse()?;
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            WaitForSingleObject(handle, 60_000);
            CloseHandle(handle);
        }
    }
    // NSIS wants /D last and unquoted, even with spaces.
    let status = std::process::Command::new(installer)
        .arg("/S")
        .raw_arg(format!("/D={install_dir}"))
        .status()
        .context(crate::i18n::t("Cannot start the installer"))?;
    ensure!(status.success(), "{}", crate::i18n::t("The installer did not finish"));
    std::process::Command::new(Path::new(install_dir).join("opentermius.exe")).spawn()?;
    Ok(())
}

/// The folder this copy was installed to, or `None` when running from a build folder.
pub fn install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    dir.join("uninstall.exe").is_file().then_some(dir)
}
