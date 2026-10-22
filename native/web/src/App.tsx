import { useEffect, useState } from "react";
import { listHosts, type Host } from "./api/hosts";
import { t } from "./i18n";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";
import { KeychainPage } from "./pages/Keychain";
import { SnippetsPage } from "./pages/Snippets";
import { KnownHostsPage } from "./pages/KnownHosts";
import { LogsPage } from "./pages/Logs";

const titles: Record<Section, string> = {
  hosts: "Hosts",
  keychain: "Keychain",
  snippets: "Snippets",
  "known-hosts": "Known Hosts",
  logs: "Logs",
  settings: "Settings",
};

export default function App({ onRun, onConnect }: { onRun?: (script: string) => void | Promise<void>; onConnect?: (hostId: string) => void | Promise<void> }) {
  const [section, setSection] = useState<Section>("hosts");
  const [hosts, setHosts] = useState<Host[]>([]);
  const [error, setError] = useState("");

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
          {section === "hosts" ? <HostsPage hosts={hosts} /> : section === "keychain" ? <KeychainPage /> : section === "snippets" ? <SnippetsPage onRun={onRun} /> : section === "known-hosts" ? <KnownHostsPage /> : section === "logs" ? <LogsPage onConnect={onConnect} /> : <div className="empty">{t(titles[section])}</div>}
        </main>
      </div>
    </div>
  );
}
