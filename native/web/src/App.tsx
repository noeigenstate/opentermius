import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { listHosts, type Host, type Usage } from "./api/hosts";
import { closeSession, connectHost, sessionInput, duplicateSession, forgetSession, latestSessionStatus, listSessions, openLocalTerminal, reconnectSession, startSessionEvents, subscribeSessions, type SessionInfo, type SessionPrompt } from "./api/sessions";
import { t, setLanguage } from "./i18n";
import { getSettings, saveSettings, onSettingsChanged, type Settings } from "./api/settings";
import { useUpdates } from "./api/updates";
import { SettingsPage } from "./pages/Settings";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";
import { PromptDialog } from "./components/PromptDialog";
import { SurfaceEffects } from "./components/SurfaceEffects";

const SessionPage = lazy(() => import("./pages/Session").then((module) => ({ default: module.SessionPage })));
import { KeychainPage } from "./pages/Keychain";
import { SnippetsPage } from "./pages/Snippets";
import { LogsPage } from "./pages/Logs";

const titles: Record<Section, string> = {
  hosts: "Hosts",
  keychain: "Keychain",
  snippets: "Snippets",
  favorites: "Favorites",
  logs: "Logs",
  settings: "Settings",
};

export default function App() {
  const [section, setSection] = useState<Section>("hosts");
  const [hosts, setHosts] = useState<Host[]>([]);
  const [error, setError] = useState("");
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [initialSessionViews, setInitialSessionViews] = useState<Record<string, "terminal" | "sftp">>({});
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [prompts, setPrompts] = useState<SessionPrompt[]>([]);
  const [usageByHost, setUsageByHost] = useState<Record<string, Usage>>({});
  const pending = useRef(new Set<string>());
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

  /** After an import or restore: hosts and preferences may both have changed. */
  async function reloadData() {
    const [nextHosts, nextSettings] = await Promise.all([listHosts(), getSettings()]);
    setHosts(nextHosts);
    applySettings(nextSettings);
  }

  useEffect(() => {
    let active = true;
    const listening = onSettingsChanged((next) => { if (active) applySettings(next); });
    listening.then(() => getSettings()).then((next) => { if (active) applySettings(next); }).catch((e) => { if (active) setError(String(e)); });
    return () => { active = false; void listening.then((unlisten) => unlisten()).catch(() => {}); };
  }, []);

  function removeSession(sessionId: string) {
    forgetSession(sessionId);
    setSessions((current) => current.filter((session) => session.sessionId !== sessionId));
    setActiveSessionId((current) => current === sessionId ? null : current);
    setPrompts((current) => current.filter((prompt) => prompt.sessionId !== sessionId));
    setInitialSessionViews((current) => { const next = { ...current }; delete next[sessionId]; return next; });
  }

  async function addSession(sessionId: string, initialView: "terminal" | "sftp" = "terminal") {
    const session = (await listSessions()).find((item) => item.sessionId === sessionId);
    if (!session) return;
    const status = latestSessionStatus(session.sessionId);
    setInitialSessionViews((current) => ({ ...current, [sessionId]: initialView }));
    setSessions((current) => [...current.filter((item) => item.sessionId !== session.sessionId), {
      ...session, ...(status ? { connected: status.connected, ended: status.ended } : {}),
    }]);
    setActiveSessionId(session.sessionId);
  }

  async function run(key: string, action: () => Promise<void>) {
    if (pending.current.has(key)) return;
    pending.current.add(key);
    setError("");
    try { await action(); } catch (reason) { setError(String(reason)); }
    finally { pending.current.delete(key); }
  }

  const reconnect = (sessionId: string) => {
    void run(sessionId, async () => {
      await reconnectSession(sessionId);
      setActiveSessionId(sessionId);
    });
  };
  /** Types a snippet into the active terminal (or the most recent one) and shows it. */
  const runSnippet = (script: string) => {
    const target = activeSessionId ?? sessions.at(-1)?.sessionId;
    if (!target) {
      setError(t("Open a terminal first, then run the snippet."));
      return;
    }
    void run(`snippet:${target}`, async () => {
      await sessionInput(target, script);
      setActiveSessionId(target);
    });
  };
  const connect = (hostId: string, initialView: "terminal" | "sftp" = "terminal") => {
    void run(`host:${hostId}`, async () => {
      await addSession(await connectHost(hostId), initialView);
    });
  };

  useEffect(() => {
    let active = true;
    const unsubscribe = subscribeSessions((event) => {
      if (event.type === "status") {
        setSessions((current) => current.map((session) => session.sessionId === event.sessionId ? { ...session, connected: event.connected, ended: event.ended } : session));
        if (event.error && event.message) setError(event.message);
        if (event.ended) setPrompts((current) => current.filter((prompt) => prompt.sessionId !== event.sessionId));
      } else if (event.type === "prompt") {
        setPrompts((current) => current.some((prompt) => prompt.promptId === event.promptId) ? current : [...current, event]);
      } else if (event.type === "stats") {
        setUsageByHost((current) => ({ ...current, [event.hostId]: { cpu: event.cpu, memory: event.memory, disk: event.disk } }));
      } else if (event.type === "closed") {
        // The backend retains ended sessions for reconnect; only explicit close removes a tab.
        setSessions((current) => current.map((session) => session.sessionId === event.sessionId ? { ...session, connected: false, ended: true } : session));
        setPrompts((current) => current.filter((prompt) => prompt.sessionId !== event.sessionId));
      }
    });
    void startSessionEvents().then(listSessions).then((items) => {
      if (active) setSessions((current) => {
        const existing = new Set(current.map((session) => session.sessionId));
        return [...current, ...items.filter((session) => !existing.has(session.sessionId)).map((session) => {
          const status = latestSessionStatus(session.sessionId);
          return { ...session, ...(status ? { connected: status.connected, ended: status.ended } : {}) };
        })];
      });
    }).catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; unsubscribe(); };
  }, []);

  const onlineHostIds = new Set(sessions.filter((session) => session.connected && session.hostId).map((session) => session.hostId!));

  useEffect(() => {
    listHosts().then(setHosts).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="app" data-glass={settings?.glassEffect ?? true}>
      <SurfaceEffects />
      <TitleBar tabs={{ sessions, activeSessionId, onSelect: (id) => { setActiveSessionId(id); if (id === null) setSection("hosts"); },
        onNew: () => { void run("local", async () => {
          await addSession(await openLocalTerminal());
        }); },
        onReconnect: reconnect,
        onDuplicate: (id) => { void run(id, async () => { await addSession(await duplicateSession(id)); }); },
        onClose: (id) => { void run(id, async () => { await closeSession(id); removeSession(id); }); },
      }} />
      <div className="body">
        <Sidebar section={section} onSelect={(next) => { setSection(next); setActiveSessionId(null); }} />
        <main className={`content${activeSessionId ? " session-content" : ""}`}>
          {error && <div className="error session-app-error" role="alert">{error}<button aria-label={t("Dismiss")} onClick={() => setError("")}>×</button></div>}
          <div id="hosts-panel" role="tabpanel" hidden={activeSessionId !== null}>
            {(section === "hosts" || section === "favorites") ? <HostsPage key={section} favoritesOnly={section === "favorites"} hosts={hosts} onHostsChange={setHosts} onConnect={connect} onSftp={(id) => connect(id, "sftp")} usageByHost={usageByHost} onlineHostIds={onlineHostIds} />
              : section === "keychain" ? <KeychainPage />
              : section === "snippets" ? <SnippetsPage onRun={runSnippet} />
              : section === "logs" ? <LogsPage visible={activeSessionId === null} onConnect={connect} />
              : section === "settings" ? <SettingsPage settings={settings} saving={saving} onChange={changeSettings} onDataChanged={reloadData} updates={updates} />
              : <div className="empty">{t(titles[section])}</div>}
          </div>
          <Suspense fallback={<div className="empty">{t("Loading terminal…")}</div>}>
            {sessions.map((session) => <SessionPage key={session.sessionId} session={session} visible={activeSessionId === session.sessionId}
              initialView={initialSessionViews[session.sessionId]} blocked={prompts.length > 0} onReconnect={() => reconnect(session.sessionId)} />)}
          </Suspense>
        </main>
      </div>
      {prompts[0] && <PromptDialog key={prompts[0].promptId} prompt={prompts[0]} onAnswered={(id) => setPrompts((current) => current.filter((prompt) => prompt.promptId !== id))} />}
    </div>
  );
}
