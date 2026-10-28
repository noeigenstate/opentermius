import { useEffect, useRef, useState, type ReactNode } from "react";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { getAppInfo, openConfigFolder, openProjectLink, type AppInfo, type Settings } from "../api/settings";
import { detectXshellFolder, exportHosts, importHostsJson, importWindterm, importXshell } from "../api/data";
import type { Updates } from "../api/updates";
import { changelog } from "../changelog";
import { t } from "../i18n";
import { Icon, type IconName } from "../icons";
import { Recovery } from "./Recovery";

/** Pages opened from the About group; each replaces the settings list until Esc or the back button. */
const subviews = {
  changelog: { icon: "list", title: "Changelog", note: "Recent improvements and fixes" },
  about: { icon: "info", title: "About OpenTermius", note: "Version, project links and configuration folder" },
} as const;
type SubView = keyof typeof subviews;
type ProjectLink = Parameters<typeof openProjectLink>[0];
const projectLinks = [["Project page", "repository"], ["Releases", "releases"], ["Report an issue", "issues"]] as const;

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

function OpenRow({ label, detail, onOpen }: { label: string; detail?: string; onOpen: () => void }) {
  return <div className="settings-row"><span>{t(label)}{detail && <small className="settings-path">{detail}</small>}</span><button className="btn" aria-label={`${t("Open")} ${t(label)}`} onClick={onOpen}>{t("Open")}</button></div>;
}

function Changelog({ current, onLink }: { current?: string; onLink: (link: ProjectLink) => void }) {
  return <div className="settings-subview">
    <div className="settings-card">{changelog.map(({ version, date, items }) => <section className="changelog-release" key={version}>
      <h2><strong>{version}</strong><time dateTime={date}>{date}</time>{version === current && <span className="chip">{t("Current")}</span>}</h2>
      <ul>{items.map(item => <li key={item}>{t(item)}</li>)}</ul>
    </section>)}</div>
    <p className="settings-hint changelog-foot"><button className="settings-link" onClick={() => onLink("releases")}>{t("All releases")}</button></p>
  </div>;
}

function About({ info, onFolder, onLink }: { info: AppInfo | null; onFolder: () => void; onLink: (link: ProjectLink) => void }) {
  return <div className="settings-subview">
    <div className="about-head">
      <span className="brand-mark"><Icon name="logo" size={34} /></span>
      <div><h2>{t("OpenTermius")}<span className="chip">{info?.version ?? "—"}</span></h2><p>{t("Local-first SSH and SFTP client")}</p></div>
    </div>
    <div className="settings-card">
      <OpenRow label="Configuration folder" detail={info?.configFolder ?? "—"} onOpen={onFolder} />
      {projectLinks.map(([label, link]) => <OpenRow key={link} label={label} onOpen={() => onLink(link)} />)}
    </div>
  </div>;
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
  const [view, setView] = useState<"main" | SubView>("main");
  const page = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<SubView | null>(null);
  useEffect(() => { getAppInfo().then(setInfo).catch(e => setError(String(e))); }, []);

  const go = (next: "main" | SubView) => { setError(""); setMessage(""); setView(next); };
  useEffect(() => {
    if (view === "main") {
      // The rows are remounted on return, so look the one that opened the page up again instead of keeping its element.
      if (opener.current) page.current?.querySelector<HTMLElement>(`[data-view="${opener.current}"]`)?.focus();
      return;
    }
    opener.current = view;
    page.current?.closest(".content")?.scrollTo(0, 0);
    heading.current?.focus({ preventScroll: true });
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); go("main"); } };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [view]);

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

  const openExternal = (action: () => Promise<void>) => void dataAction(async () => { await action(); return null; });
  const openLink = (link: ProjectLink) => openExternal(() => openProjectLink(link));

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

  return <div className="page settings-page" ref={page}>
    <header className="page-header">
      {view === "main" ? <>
        <div><h1>{t("Settings")}</h1><p>{t("Customize your workspace and connections.")}</p></div>
        <div className="page-actions"><span className="settings-save-state" data-saving={saving} role="status"><i className={`dot${saving ? " dot-busy" : " dot-online"}`} />{saving ? t("Saving…") : t("Changes save automatically")}</span></div>
      </> : <div>
        <button className="btn btn-ghost settings-back" onClick={() => go("main")}><Icon name="chevronLeft" size={14} />{t("Settings")}</button>
        <h1 ref={heading} tabIndex={-1}>{t(subviews[view].title)}</h1>
      </div>}
    </header>
    {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
    {message && <div className="banner banner-ok" role="status"><Icon name="check" size={16} /><span>{message}</span></div>}
    {view === "changelog" && <Changelog current={info?.version} onLink={openLink} />}
    {view === "about" && <About info={info} onFolder={() => openExternal(openConfigFolder)} onLink={openLink} />}
    {view === "main" && <fieldset className="settings-fields" disabled={saving || busy || recovery}>
      <Group title="Appearance" icon="settings">
        <label className="settings-row"><span>{t("Language")}</span><select aria-label={t("Language")} value={settings.language} onChange={e => change("language", e.target.value as Settings["language"])}><option value="en">{t("English")}</option><option value="zh-CN">{t("Simplified Chinese")}</option><option value="system">{t("System")}</option></select></label>
        <label className="settings-row"><span>{t("Theme")}</span><select aria-label={t("Theme")} value={settings.themeSchedule ? "schedule" : settings.appearance} onChange={e => {
          const value = e.target.value;
          setError("");
          void onChange({ ...settings, appearance: value === "schedule" ? settings.appearance : value as Settings["appearance"], themeSchedule: value === "schedule" }).catch(e => setError(String(e)));
        }}><option value="light">{t("White background, black text")}</option><option value="dark">{t("Black background, white text")}</option><option value="system">{t("Follow system")}</option><option value="schedule">{t("Day/night by time")}</option></select></label>
        {settings.themeSchedule && <>
          <label className="settings-row"><span>{t("Light from")}</span><input type="time" aria-label={t("Light from")} value={settings.themeDayStart} onChange={e => change("themeDayStart", e.target.value)} /></label>
          <label className="settings-row"><span>{t("Dark from")}</span><input type="time" aria-label={t("Dark from")} value={settings.themeNightStart} onChange={e => change("themeNightStart", e.target.value)} /></label>
        </>}
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
        <Toggle label="Check for updates automatically" value={settings.autoUpdate} onChange={v => change("autoUpdate", v)} />
        <div className="settings-actions"><button className="btn" disabled={updates.busy} onClick={() => void updates.check()}>{t("Check now")}</button>{updates.result?.version && <button className="btn btn-primary" disabled={updates.busy} onClick={() => void updates.install(updates.result!.version!)}>{t("Update to {version}", { version: updates.result.version })}</button>}</div>
        <p className="settings-hint" role="status">{updates.busy ? t(updates.installing ? "Downloading and verifying update…" : "Checking for updates…") : updates.result?.upToDate ? t("OpenTermius is up to date") : updates.result?.version ? t("Version {version} is available", { version: updates.result.version }) : t("Build copies open the release page for manual installation.")}</p>
        {updates.error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{updates.error}</span></div>}
        {(Object.keys(subviews) as SubView[]).map(key => <button key={key} className="settings-row settings-nav" data-view={key} onClick={() => go(key)}>
          <Icon name={subviews[key].icon} size={20} /><span><strong>{t(subviews[key].title)}</strong><small>{t(subviews[key].note)}</small></span><Icon name="chevronRight" />
        </button>)}
      </Group>
    </fieldset>}
    {busy && <p className="feature-loading" role="status">{t("Working…")}</p>}
    {recovery && <Recovery onClose={() => setRecovery(false)} onRestored={onDataChanged} />}
  </div>;
}
