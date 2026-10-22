//! Update metadata parsing and installer verification; no network unless opted in.
use opentermius::update::{self, Checksum};

const RELEASE: &str = r#"{
    "tag_name": "v1.4.0",
    "html_url": "https://example.invalid/releases/tag/v1.4.0",
    "assets": [
        {"name": "checksums.json", "browser_download_url": "https://example.invalid/checksums.json"},
        {"name": "OpenTermius-1.4.0-windows-x64-setup.exe", "browser_download_url": "https://example.invalid/setup.exe"},
        {"name": "notes.txt", "browser_download_url": "https://example.invalid/notes.txt"}
    ]
}"#;

#[test]
fn versions_compare_numerically() {
    assert_eq!(update::parse_version("v0.3.10"), Some((0, 3, 10)));
    assert_eq!(update::parse_version("1.2"), Some((1, 2, 0)));
    assert_eq!(update::parse_version("2.0.0-beta.1"), Some((2, 0, 0)));
    assert_eq!(update::parse_version("1.2.3.4"), None);
    assert_eq!(update::parse_version("latest"), None);
    assert!(update::is_newer("0.3.2", "0.3.10"));
    assert!(update::is_newer("0.3.2", "v1.0.0"));
    assert!(!update::is_newer("0.3.2", "0.3.2"));
    assert!(!update::is_newer("0.4.0", "0.3.9"));
    assert!(!update::is_newer("0.3.2", "garbage"));
}

#[test]
fn release_picks_installer_and_checksums() {
    let release = update::parse_release(RELEASE).unwrap();
    assert_eq!(release.version, "1.4.0");
    assert_eq!(release.installer_name, "OpenTermius-1.4.0-windows-x64-setup.exe");
    assert_eq!(release.installer_url, "https://example.invalid/setup.exe");
    assert_eq!(release.checksums_url, "https://example.invalid/checksums.json");
    assert!(update::parse_release(&RELEASE.replace("windows-x64-setup.exe", "linux.tar.gz")).is_err());
    assert!(update::parse_release(&RELEASE.replace("https://example.invalid/setup.exe", "http://example.invalid/setup.exe")).is_err());
    assert!(update::parse_release(&RELEASE.replace("\"checksums.json\"", "\"sums.txt\"")).is_err());
    assert!(update::parse_release("{}").is_err());
}

#[test]
fn installer_must_match_name_size_and_hash() {
    let installer = b"synthetic installer bytes";
    let checksum = update::parse_checksum(&format!(
        r#"{{"version":"1.4.0","file":"setup.exe","bytes":{},"sha256":"{}"}}"#,
        installer.len(),
        update::sha256_hex(installer).to_uppercase()
    ))
    .unwrap();
    assert!(update::verify(installer, "setup.exe", &checksum).is_ok());
    assert!(update::verify(installer, "other.exe", &checksum).is_err());
    assert!(update::verify(b"synthetic installer byteZ", "setup.exe", &checksum).is_err());
    let short = Checksum { bytes: 3, ..checksum };
    assert!(update::verify(installer, "setup.exe", &short).is_err());
    assert!(update::parse_checksum(r#"{"file":"setup.exe","bytes":1,"sha256":"abc"}"#).is_err());
    assert!(update::parse_checksum("not json").is_err());
    assert_eq!(
        update::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// Opt-in: `OPENTERMIUS_LIVE_UPDATE_CHECK=1` queries the real latest release.
#[test]
#[ignore]
fn live_latest_release_is_well_formed() {
    if std::env::var_os("OPENTERMIUS_LIVE_UPDATE_CHECK").is_none() {
        return;
    }
    let latest = update::check("0.0.1").unwrap().expect("a release newer than 0.0.1");
    println!("latest release {} → {}", latest.version, latest.installer_name);
    assert!(update::check("999.0.0").unwrap().is_none());
}
