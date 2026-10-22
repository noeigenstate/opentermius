import { useEffect, useState } from "react";
import { listHosts, type Host } from "./api/hosts";
import { t, setLanguage } from "./i18n";
import { getSettings, saveSettings, onSettingsChanged, type Settings } from "./api/settings";
import { useUpdates } from "./api/updates";
import { SettingsPage } from "./pages/Settings";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";

const titles: Record<Section, string> = {
  hosts: "Hosts",
  keychain: "Keychain",
  snippets: "Snippets",
  "known-hosts": "Known Hosts",
  logs: "Logs",
  settings: "Settings",
};

export default function App() {
  const [section, setSection] = useState<Section>("hosts");
  const [hosts, setHosts] = useState<Host[]>([]);
  const [error, setError] = useState("");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const updates = useUpdates(settings?.autoUpdate ?? false);

  function applySettings(next: Settings) {
    setLanguage(next.language);
    setSettings(next);
  }

  async function changeSettings(next: Settings) {
    setSaving(true);
    try { applySettings(await saveSettings(next)); }
    finally { setSaving(false); }
  }

  async function reloadData() {
    const [nextHosts, nextSettings] = await Promise.all([listHosts(), getSettings()]);
    setHosts(nextHosts);
    applySettings(nextSettings);
  }

  useEffect(() => {
    let active = true;
    const listening = onSettingsChanged(next => { if (active) applySettings(next); });
    listening.then(() => getSettings()).then(next => { if (active) applySettings(next); }).catch(e => { if (active) setError(String(e)); });
    return () => { active = false; void listening.then(unlisten => unlisten()).catch(() => {}); };
  }, []);

  useEffect(() => {
    listHosts().then(setHosts).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="app">
      <TitleBar title={titles[section]} />
      <div className="body">
        <Sidebar section={section} onSelect={setSection} />
        <main className="content">
          {error && <div className="error">{error}</div>}
          {section === "hosts" ? <HostsPage hosts={hosts} /> : section === "settings" ? <SettingsPage settings={settings} saving={saving} onChange={changeSettings} onDataChanged={reloadData} updates={updates} /> : <div className="empty">{t(titles[section])}</div>}
        </main>
      </div>
    </div>
  );
}
