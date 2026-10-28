/** Release notes shown in Settings, newest first. Items are English source sentences; the page passes each through `t()`. */
export const changelog: { version: string; date: string; items: string[] }[] = [
  { version: "0.8.6", date: "2026-10-28", items: [
    "Settings now has a changelog and an About page.",
    "Each release now includes macOS (Apple silicon) and Linux (x64 and arm64 AppImage) packages.",
    "On macOS, one Terminal command installs or updates OpenTermius without the security prompt.",
  ] },
  { version: "0.8.5", date: "2026-10-28", items: [
    "The SFTP transfer panel appears only while transfers run, shows up to three, and keeps failures in the error banner.",
  ] },
  { version: "0.8.4", date: "2026-10-28", items: [
    "The SFTP local path has a places menu with home, desktop, downloads, documents and drives.",
    "SFTP files show icons by file type.",
    "Right-clicking in the terminal copies the selection and no longer pastes.",
  ] },
  { version: "0.8.3", date: "2026-10-27", items: [
    "Update checks use the Windows system proxy and SOCKS proxies, and errors show the real cause.",
    "SFTP files have a right-click menu to transfer, copy, rename and delete.",
  ] },
  { version: "0.8.2", date: "2026-10-27", items: [
    "Removed the black frame around terminal text.",
  ] },
  { version: "0.8.1", date: "2026-10-27", items: [
    "Terminal selections can start in the margin, and double-click selection stops at CJK punctuation.",
    "Paste works on first use without a clipboard prompt.",
    "The last output line stays inside the terminal card.",
  ] },
];
