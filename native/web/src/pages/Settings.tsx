import { useEffect, useState, type ReactNode } from "react";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { getAppInfo, openConfigFolder, type AppInfo, type Settings } from "../api/settings";
import { detectXshellFolder, exportHosts, importHostsJson, importWindterm, importXshell } from "../api/data";
import type { Updates } from "../api/updates";
import { t } from "../i18n";
import { Icon, type IconName } from "../icons";
import { Recovery } from "./Recovery";

function Group({ title, icon, children }: { title: string; icon: IconName; children: ReactNode }) {
  return <section className="settings-group" aria-label={t(title)}><h2><Icon name={icon} />{t(title)}</h2><div className="settings-card">{children}</div></section>;
}

function Toggle({ label, value, onChange }: { label: string; value: boolean; onChange: (value: boolean) => void }) {
  return <label className="settings-row"><span>{t(label)}</span><input type="checkbox" role="switch" checked={value} onChange={e => onChange(e.target.checked)} /></label>;
}

function NumberSetting({ label, value, min, max, step = 1, onChange }: { label: string; value: number; min: number; max: number; step?: number; onChange: (value: number) => void }) {
  // Keep an editable draft so typing a multi-digit number does not persist each partial digit.
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  function commit() {
    const parsed = Number(draft);
    if (!draft.trim() || !Number.isFinite(parsed)) { setDraft(String(value)); return; }
    const next = Math.max(min, Math.min(max, Math.round(parsed)));
    setDraft(String(next));
    if (next !== value) onChange(next);
  }
  return <label className="settings-row"><span>{t(label)}</span><input type="number" min={min} max={max} step={step} value={draft} onChange={e => setDraft(e.target.value)} onBlur={commit} onKeyDown={e => { if (e.key === "Enter") e.currentTarget.blur(); }} /></label>;
}

export function SettingsPage({ settings, saving, onChange, onDataChanged, updates }: {
  settings: Settings | null; saving: boolean;
  onChange: (settings: Settings) => Promise<void>;
  onDataChanged: () => Promise<void>;
  updates: Updates;
}) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [recovery, setRecovery] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  useEffect(() => { getAppInfo().then(setInfo).catch(e => setError(String(e))); }, []);

  const change = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    if (!settings) return;
    setError("");
    void onChange({ ...settings, [key]: value }).catch(e => setError(String(e)));
  };

  async function dataAction(action: () => Promise<string | null>, refresh = false) {
    setBusy(true); setError(""); setMessage("");
    try {
      const result = await action();
      if (result) {
        setMessage(result);
        if (refresh) await onDataChanged();
      }
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function importJson() {
    const path = await open({ title: t("Import hosts JSON"), filters: [{ name: "JSON", extensions: ["json"] }] });
    return path ? importHostsJson(path) : null;
  }

  async function xshell() {
    const defaultFolder = await detectXshellFolder();
    const useDefault = defaultFolder && await ask(t("Use Xshell sessions from {path}, or choose another folder?", { path: defaultFolder }), {
      title: t("Import from Xshell"), okLabel: t("Use default folder"), cancelLabel: t("Choose folder"),
    });
    if (useDefault) return importXshell(defaultFolder);
    const path = await open({ title: t("Select Xshell Sessions folder"), directory: true, defaultPath: defaultFolder ?? undefined });
    return path ? importXshell(path) : null;
  }

  async function windterm() {
    let { files } = await importWindterm();
    if (files.length) {
      const accepted = await ask(t("Import WindTerm sessions from these files?\n{files}", { files: files.join("\n") }), {
        title: t("Import from WindTerm"), okLabel: t("Import"), cancelLabel: t("Choose files"),
      });
      if (!accepted) files = [];
    }
    if (!files.length) {
      const selection = await open({ title: t("Select WindTerm user.sessions files"), multiple: true });
      if (!selection?.length) return null;
      files = selection;
      if (!await ask(t("Import WindTerm sessions from these files?\n{files}", { files: files.join("\n") }), {
        title: t("Import from WindTerm"), okLabel: t("Import"), cancelLabel: t("Cancel"),
      })) return null;
    }
    return (await importWindterm(files)).summary;
  }

  async function exportJson() {
    const path = await save({ title: t("Export hosts (without credentials)"), defaultPath: "opentermius-hosts.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    return path ? exportHosts(path) : null;
  }

  if (!settings) return <div className="empty" role="status">{t("Loading settings…")}</div>;

  return <div className="page settings-page">
    <header className="page-header">
      <div><h1>{t("Settings")}</h1><p>{t("Customize your workspace and connections.")}</p></div>
      <div className="page-actions"><span className="settings-save-state" data-saving={saving} role="status"><i className={`dot${saving ? " dot-busy" : " dot-online"}`} />{saving ? t("Saving…") : t("Changes save automatically")}</span></div>
    </header>
    {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
    {message && <div className="banner banner-ok" role="status"><Icon name="check" size={16} /><span>{message}</span></div>}
    <fieldset className="settings-fields" disabled={saving || busy || recovery}>
      <Group title="Appearance" icon="settings">
        <label className="settings-row"><span>{t("Language")}</span><select aria-label={t("Language")} value={settings.language} onChange={e => change("language", e.target.value as Settings["language"])}><option value="en">{t("English")}</option><option value="zh-CN">{t("Simplified Chinese")}</option><option value="system">{t("System")}</option></select></label>
      </Group>
      <Group title="Terminal" icon="terminal">
        <NumberSetting label="Font size" value={settings.fontSize} min={10} max={24} onChange={v => change("fontSize", v)} />
        <NumberSetting label="Scrollback lines" value={settings.scrollbackLines} min={1000} max={100000} step={1000} onChange={v => change("scrollbackLines", v)} />
        <Toggle label="Cursor blink" value={settings.cursorBlink} onChange={v => change("cursorBlink", v)} />
        <Toggle label="Copy on select" value={settings.copyOnSelect} onChange={v => change("copyOnSelect", v)} />
        <Toggle label="Confirm multi-line paste" value={settings.confirmMultilinePaste} onChange={v => change("confirmMultilinePaste", v)} />
        <label className="settings-row"><span>{t("Bell")}</span><select aria-label={t("Bell")} value={settings.bell} onChange={e => change("bell", e.target.value as Settings["bell"])}><option value="none">{t("None")}</option><option value="visual">{t("Visual")}</option></select></label>
      </Group>
      <Group title="SSH" icon="hosts">
        <NumberSetting label="Keepalive (seconds, 0 disables)" value={settings.keepaliveSeconds} min={0} max={3600} onChange={v => change("keepaliveSeconds", v)} />
        <NumberSetting label="Connection timeout (seconds)" value={settings.connectionTimeoutSeconds} min={1} max={300} onChange={v => change("connectionTimeoutSeconds", v)} />
      </Group>
      <Group title="SFTP" icon="snippets">
        <Toggle label="Confirm overwrite" value={settings.confirmOverwrite} onChange={v => change("confirmOverwrite", v)} />
        <Toggle label="Show hidden files" value={settings.showHiddenFiles} onChange={v => change("showHiddenFiles", v)} />
      </Group>
      <Group title="Data" icon="keychain">
        <div className="settings-actions"><button className="btn" onClick={() => void dataAction(importJson, true)}>{t("Import hosts JSON")}</button><button className="btn" onClick={() => void dataAction(xshell, true)}>{t("Import from Xshell")}</button><button className="btn" onClick={() => void dataAction(windterm, true)}>{t("Import from WindTerm")}</button><button className="btn" onClick={() => void dataAction(exportJson)}>{t("Export hosts")}</button><button className="btn" onClick={() => setRecovery(true)}>{t("Backup & Restore")}</button></div>
        <p className="settings-hint">{t("Host imports and exports exclude credentials. Use a recovery code for a complete backup.")}</p>
      </Group>
      <Group title="About" icon="hosts">
        <div className="settings-row"><span>{t("Version")}</span><span className="chip">{info?.version ?? "—"}</span></div>
        <div className="settings-row"><span>{t("Configuration folder")}<small className="settings-path">{info?.configFolder ?? "—"}</small></span><button className="btn" onClick={() => void dataAction(async () => { await openConfigFolder(); return null; })}>{t("Open")}</button></div>
        <Toggle label="Check for updates automatically" value={settings.autoUpdate} onChange={v => change("autoUpdate", v)} />
        <div className="settings-actions"><button className="btn" disabled={updates.busy} onClick={() => void updates.check()}>{t("Check now")}</button>{updates.result?.version && <button className="btn btn-primary" disabled={updates.busy} onClick={() => void updates.install(updates.result!.version!)}>{t("Update to {version}", { version: updates.result.version })}</button>}</div>
        <p className="settings-hint" role="status">{updates.busy ? t(updates.installing ? "Downloading and verifying update…" : "Checking for updates…") : updates.result?.upToDate ? t("OpenTermius is up to date") : updates.result?.version ? t("Version {version} is available", { version: updates.result.version }) : t("Build copies open the release page for manual installation.")}</p>
        {updates.error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{updates.error}</span></div>}
      </Group>
    </fieldset>
    {busy && <p className="feature-loading" role="status">{t("Working…")}</p>}
    {recovery && <Recovery onClose={() => setRecovery(false)} onRestored={onDataChanged} />}
  </div>;
}
