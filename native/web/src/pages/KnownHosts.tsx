import { useEffect, useState } from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import * as api from "../api/knownHosts";
import { t } from "../i18n";
import { Icon } from "../icons";

export function KnownHostsPage() {
  const [hosts, setHosts] = useState<api.KnownHost[]>([]);
  const [search, setSearch] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const refresh = () => api.listKnownHosts().then(setHosts);
  useEffect(() => { refresh().catch(e => setError(String(e))).finally(() => setLoading(false)); }, []);
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError(""); setMessage("");
    try { await action(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function importFile() {
    await act(async () => {
      const path = await open({ title: t("Import known_hosts"), directory: false, multiple: false });
      if (!path) return;
      const result = await api.importKnownHosts(path);
      setMessage(t("Imported {added}; already existed {unchanged}; conflicts kept unchanged {conflicts}; skipped {skipped}.", { ...result }));
      await refresh();
    });
  }

  async function removeHost(host: api.KnownHost) {
    await act(async () => {
      const accepted = await confirm(t("Remove trust for {host}:{port}? You will be asked to verify its fingerprint on the next connection.", {
        host: host.host, port: host.port,
      }), { title: t("Remove trusted host"), kind: "warning" });
      if (!accepted) return;
      await api.deleteKnownHost(host.host, host.port);
      await refresh();
    });
  }
  const query = search.trim().toLowerCase();
  const visible = hosts.filter(host => [host.host, String(host.port), host.fingerprint, ...host.usedBy].some(value => value.toLowerCase().includes(query)));
  return <section className="page feature-page">
    <header className="feature-header"><div className="feature-heading"><h1>{t("Known Hosts")}</h1><p>{t("Review the fingerprints of hosts you trust.")}</p></div><label className="search"><Icon name="search" /><input aria-label={t("Search known hosts")} placeholder={t("Search known hosts")} value={search} onChange={e => setSearch(e.target.value)} /></label><button className="feature-button" disabled={busy} onClick={() => void act(refresh)}>{t("Refresh")}</button><button className="feature-button primary" data-depth="true" disabled={busy} onClick={() => void importFile()}>{t("Import")}</button></header>
    {error && <div className="feature-error" role="alert">{error}</div>}{message && <p className="feature-notice" role="status">{message}</p>}
    {loading ? <p role="status">{t("Loading…")}</p> : visible.length === 0 ? <div className="feature-empty"><span className="feature-empty-symbol"><Icon name="known" size={34} /></span><h2>{t(hosts.length ? "No matching hosts" : "No known hosts")}</h2><p>{t("Hosts you trust on first connect appear here.")}</p><button className="feature-button primary" data-depth="true" disabled={busy} onClick={() => void importFile()}>{t("Import known_hosts")}</button></div> : <div className="feature-table-wrap"><table className="feature-table"><thead><tr><th>{t("Host:port")}</th><th>{t("SHA-256 fingerprint")}</th><th>{t("Saved hosts")}</th><th>{t("Actions")}</th></tr></thead><tbody>
      {visible.map(host => <tr key={JSON.stringify([host.host, host.port])}>
        <td className="feature-mono">{host.host.includes(":") ? `[${host.host}]` : host.host}:{host.port}</td>
        <td className="feature-mono feature-wrap">{host.fingerprint}</td>
        <td>{host.usedBy.join(", ") || t("Not in saved hosts")}</td>
        <td><button className="feature-button danger" disabled={busy} onClick={() => void removeHost(host)}>{t("Delete")}</button></td>
      </tr>)}
    </tbody></table></div>}
  </section>;
}
