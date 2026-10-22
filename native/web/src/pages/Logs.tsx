import { useEffect, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import * as api from "../api/logs";
import { t } from "../i18n";
import { Icon } from "../icons";

const statuses = { connecting: "Connecting", connected: "Connected", closed: "Closed", failed: "Failed" } as const;

function duration(entry: api.LogEntry, now: number) {
  const seconds = Math.max(0, Math.floor(((entry.endedAt ? Date.parse(entry.endedAt) : now) - Date.parse(entry.startedAt)) / 1000));
  if (!Number.isFinite(seconds)) return "—";
  if (seconds < 60) return t("{seconds}s", { seconds });
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return t("{minutes}m {seconds}s", { minutes, seconds: seconds % 60 });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t("{hours}h {minutes}m", { hours, minutes: minutes % 60 });
  return t("{days}d {hours}h", { days: Math.floor(hours / 24), hours: hours % 24 });
}

export function LogsPage({ onConnect, visible: pageVisible = true }: { onConnect?: (hostId: string) => void | Promise<void>; visible?: boolean }) {
  const [entries, setEntries] = useState<api.LogEntry[]>([]);
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());
  const refresh = () => api.listLogs().then(setEntries);
  useEffect(() => {
    if (!pageVisible) return;
    let active = true;
    let fetching = false;
    let timer: number | undefined;
    const update = async () => {
      if (fetching || !active || document.visibilityState !== "visible") return;
      fetching = true;
      try { const rows = await api.listLogs(); if (active) { setEntries(rows); setError(""); } }
      catch (e) { if (active) setError(String(e)); }
      finally { fetching = false; if (active) setLoading(false); }
    };
    const schedule = () => {
      window.clearInterval(timer);
      timer = undefined;
      if (document.visibilityState !== "visible") return;
      setNow(Date.now());
      void update();
      timer = window.setInterval(() => { setNow(Date.now()); void update(); }, 5000);
    };
    schedule();
    document.addEventListener("visibilitychange", schedule);
    return () => { active = false; window.clearInterval(timer); document.removeEventListener("visibilitychange", schedule); };
  }, [pageVisible]);
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await action(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }

  async function clearHistory() {
    await act(async () => {
      const accepted = await confirm(t("Clear all connection history? This cannot be undone."), {
        title: t("Clear logs"), kind: "warning",
      });
      if (!accepted) return;
      await api.clearLogs();
      await refresh();
    });
  }
  const query = search.trim().toLowerCase();
  const visible = entries.filter(entry => (!status || entry.status === status) && [entry.label, entry.address, entry.kind, entry.status, t(statuses[entry.status]), entry.error || ""].some(value => value.toLowerCase().includes(query)));
  return <section className="page feature-page">
    <header className="feature-header">
      <div className="feature-heading"><h1>{t("Logs")}</h1><p>{t("Review recent connections and session activity.")}</p></div>
      <label className="search"><Icon name="search" /><input aria-label={t("Filter logs")} placeholder={t("Filter logs")} value={search} onChange={e => setSearch(e.target.value)} /></label>
      <label className="feature-filter"><span>{t("Status")}</span><select value={status} onChange={e => setStatus(e.target.value)}><option value="">{t("All")}</option>{Object.entries(statuses).map(([value, label]) => <option key={value} value={value}>{t(label)}</option>)}</select></label>
      <button className="feature-button danger" disabled={busy || !entries.length} onClick={() => void clearHistory()}>{t("Clear logs")}</button>
    </header>
    {error && <div className="feature-error" role="alert">{error}</div>}
    {loading ? <p role="status">{t("Loading…")}</p> : visible.length === 0 ? <div className="feature-empty"><span className="feature-empty-symbol"><Icon name="logs" size={34} /></span><h2>{t(entries.length ? "No matching sessions" : "No log entries")}</h2><p>{t("Connect to a host or open a terminal to see it here.")}</p><button className="feature-button" disabled={busy} onClick={() => { setSearch(""); setStatus(""); void act(refresh); }}>{t("Refresh")}</button></div> : <div className="feature-table-wrap"><table className="feature-table feature-log-table"><thead><tr>{["Time", "Host", "Address", "Protocol", "Status", "Duration", "Error", "Actions"].map(label => <th key={label}>{t(label)}</th>)}</tr></thead><tbody>
      {visible.map(entry => <tr key={entry.id}>
        <td>{new Date(entry.startedAt).toLocaleString()}</td>
        <td>{entry.kind === "local" ? t("Local Terminal") : entry.label}</td>
        <td className="feature-mono feature-wrap">{entry.kind === "local" ? t("Local") : entry.address}</td>
        <td>{entry.kind === "ssh" ? "SSH" : t("Local")}</td>
        <td><span className={`feature-status status-${entry.status}`}>{t(statuses[entry.status])}</span></td>
        <td>{duration(entry, now)}</td>
        <td className="feature-log-error">{entry.error || "—"}</td>
        <td>{entry.canConnect && <button className="feature-button" disabled={busy || !onConnect} title={!onConnect ? t("Session connection is not available") : undefined} onClick={() => void act(async () => { await onConnect?.(entry.hostId); })}>{t("Connect again")}</button>}</td>
      </tr>)}
    </tbody></table></div>}
  </section>;
}
