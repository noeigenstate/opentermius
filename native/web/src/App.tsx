import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { listHosts, type Host, type Usage } from "./api/hosts";
import { closeSession, connectHost, sessionInput, duplicateSession, forgetSession, latestSessionStatus, listSessions, openLocalTerminal, reconnectSession, startSessionEvents, subscribeSessions, type SessionInfo, type SessionPrompt } from "./api/sessions";
import { t, setLanguage } from "./i18n";
import { Icon } from "./icons";
import { getSettings, saveSettings, onSettingsChanged, type Settings } from "./api/settings";
import { useUpdates } from "./api/updates";
import { SettingsPage } from "./pages/Settings";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";
import { TerminalsPage } from "./pages/Terminals";
import { PromptDialog } from "./components/PromptDialog";
import { SurfaceEffects } from "./components/SurfaceEffects";

const SessionPage = lazy(() => import("./pages/Session").then((module) => ({ default: module.SessionPage })));
import { KeychainPage } from "./pages/Keychain";
import { SnippetsPage } from "./pages/Snippets";
import { LogsPage } from "./pages/Logs";

const titles: Record<Section, string> = {
  hosts: "Hosts",
  terminals: "Terminals",
  keychain: "Keychain",
  snippets: "Snippets",
  favorites: "Favorites",
  logs: "Logs",
  settings: "Settings",
};

export function hostConnectionStates(sessions: readonly SessionInfo[], failedHostIds: ReadonlySet<string>): Map<string, "connected" | "lost"> {
  const states = new Map<string, "connected" | "lost">();
  for (const hostId of failedHostIds) states.set(hostId, "lost");
  for (const session of sessions) {
    if (!session.hostId || session.local) continue;
    if (session.connected) states.set(session.hostId, "connected");
    // Native failures and transport closure set ended; false/false is still connecting.
    else if (session.ended && states.get(session.hostId) !== "connected") states.set(session.hostId, "lost");
  }
  return states;
}

export default function App() {
  const [section, setSection] = useState<Section>("hosts");
  const [hosts, setHosts] = useState<Host[]>([]);
  const [error, setError] = useState("");
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [failedHostIds, setFailedHostIds] = useState(new Set<string>());
  const [initialSessionViews, setInitialSessionViews] = useState<Record<string, "terminal" | "sftp">>({});
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const lastSessionId = useRef<string | null>(null);
  const sessionState = useRef({ sessions, activeSessionId });
  const [prompts, setPrompts] = useState<SessionPrompt[]>([]);
  const [usageByHost, setUsageByHost] = useState<Record<string, Usage>>({});
  const pending = useRef(new Set<string>());
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const updates = useUpdates(settings?.autoUpdate ?? false);

  useEffect(() => { sessionState.current = { sessions, activeSessionId }; if (activeSessionId) lastSessionId.current = activeSessionId; }, [sessions, activeSessionId]);

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
    const { sessions, activeSessionId } = sessionState.current;
    const index = sessions.findIndex((session) => session.sessionId === sessionId);
    const nextSessionId = sessions[index + 1]?.sessionId ?? sessions[index - 1]?.sessionId ?? null;
    const hostId = sessions[index]?.hostId;
    setFailedHostIds((current) => { const next = new Set(current); if (hostId) next.delete(hostId); return next; });
    forgetSession(sessionId);
    setSessions((current) => current.filter((session) => session.sessionId !== sessionId));
    setActiveSessionId((current) => current === sessionId ? nextSessionId : current);
    if (activeSessionId === sessionId) setSection("terminals");
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

  async function run(key: string, action: () => Promise<void>, hostId?: string) {
    if (pending.current.has(key)) return;
    pending.current.add(key);
    setError("");
    try { await action(); } catch (reason) {
      if (hostId) setFailedHostIds((current) => new Set(current).add(hostId));
      setError(String(reason));
    }
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
  const openLocal = () => {
    void run("local", async () => {
      await addSession(await openLocalTerminal());
    });
  };
  const connect = (hostId: string, initialView: "terminal" | "sftp" = "terminal") => {
    void run(`host:${hostId}`, async () => {
      await addSession(await connectHost(hostId), initialView);
      setFailedHostIds((current) => { const next = new Set(current); next.delete(hostId); return next; });
    }, hostId);
  };
  /** Shows the host's newest connected session; connects when it has none. */
  const openHostSession = (hostId: string) => {
    const session = sessions.filter((item) => item.hostId === hostId && !item.local && item.connected).at(-1);
    if (session) setActiveSessionId(session.sessionId);
    else connect(hostId);
  };

  useEffect(() => {
    let active = true;
    const unsubscribe = subscribeSessions((event) => {
      if (event.type === "status") {
        if (event.hostsChanged) void listHosts().then((items) => { if (active) setHosts(items); }).catch((reason) => { if (active) setError(String(reason)); });
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

  const connectionStates = hostConnectionStates(sessions, failedHostIds);

  useEffect(() => {
    setFailedHostIds((current) => {
      const next = new Set(current);
      for (const session of sessions) if (session.connected) next.delete(session.hostId);
      return next.size === current.size ? current : next;
    });
  }, [sessions]);

  useEffect(() => {
    listHosts().then(setHosts).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="app">
      <SurfaceEffects />
      <div className="app-body">
        <Sidebar section={activeSessionId ? "terminals" : section} onSelect={(next) => { setSection(next); setActiveSessionId(next === "terminals" ? sessions.find((session) => session.sessionId === lastSessionId.current)?.sessionId ?? sessions.at(-1)?.sessionId ?? null : null); }} />
        <div className="workspace-shell">
          {/* Host management and terminals stay apart: the Hosts page enters sessions from its cards, not from tabs. */}
          <TitleBar tabs={!activeSessionId && (section === "hosts" || section === "favorites") ? undefined : { sessions, activeSessionId, onSelect: setActiveSessionId,
            onNew: openLocal,
            onReconnect: reconnect,
            onDuplicate: (id) => { void run(id, async () => { await addSession(await duplicateSession(id)); }); },
            onClose: (id) => { void run(id, async () => { await closeSession(id); removeSession(id); }); },
          }} />
          <main className={`content${activeSessionId ? " session-content" : ""}`}>
            {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span><button className="icon-button icon-button-sm" aria-label={t("Dismiss")} onClick={() => setError("")}><Icon name="close" size={14} /></button></div>}
            <div id="hosts-panel" hidden={activeSessionId !== null}>
              {(section === "hosts" || section === "favorites") ? <HostsPage key={section} favoritesOnly={section === "favorites"} hosts={hosts} onHostsChange={setHosts} onConnect={connect} onOpenSession={openHostSession} onSftp={(id) => connect(id, "sftp")} onLocalTerminal={openLocal} usageByHost={usageByHost} connectionStates={connectionStates} />
                : section === "terminals" ? <TerminalsPage onLocalTerminal={openLocal} onHosts={() => setSection("hosts")} />
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
      </div>
      {prompts[0] && <PromptDialog key={prompts[0].promptId} prompt={prompts[0]} onAnswered={(id) => setPrompts((current) => current.filter((prompt) => prompt.promptId !== id))} />}
    </div>
  );
}
