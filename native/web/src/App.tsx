import { useEffect, useState } from "react";
import { listHosts, type Host } from "./api/hosts";
import { t } from "./i18n";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";
import { SftpView } from "./pages/Sftp";

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
  const [sftpSessionId, setSftpSessionId] = useState(() => new URLSearchParams(location.hash.slice(1)).get("sftp"));

  useEffect(() => {
    const changed = () => setSftpSessionId(new URLSearchParams(location.hash.slice(1)).get("sftp"));
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);

  useEffect(() => {
    listHosts().then(setHosts).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="app">
      <TitleBar title={sftpSessionId ? "SFTP" : titles[section]} />
      <div className="body">
        <Sidebar section={section} onSelect={(next) => { setSection(next); location.hash = ""; }} />
        <main className="content">
          {error && <div className="error">{error}</div>}
          {sftpSessionId ? <SftpView sessionId={sftpSessionId} /> : section === "hosts" ? <HostsPage hosts={hosts} /> : <div className="empty">{t(titles[section])}</div>}
        </main>
      </div>
    </div>
  );
}
