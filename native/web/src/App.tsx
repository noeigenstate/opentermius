import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { listHosts, type Host, type Usage } from "./api/hosts";
import { closeSession, connectHost, duplicateSession, forgetSession, latestSessionStatus, listSessions, openLocalTerminal, reconnectSession, startSessionEvents, subscribeSessions, type SessionInfo, type SessionPrompt } from "./api/sessions";
import { t } from "./i18n";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { HostsPage } from "./pages/Hosts";
import { PromptDialog } from "./components/PromptDialog";

const SessionPage = lazy(() => import("./pages/Session").then((module) => ({ default: module.SessionPage })));

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
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [prompts, setPrompts] = useState<SessionPrompt[]>([]);
  const [usageByHost, setUsageByHost] = useState<Record<string, Usage>>({});
  const pending = useRef(new Set<string>());

  function removeSession(sessionId: string) {
    forgetSession(sessionId);
    setSessions((current) => current.filter((session) => session.sessionId !== sessionId));
    setActiveSessionId((current) => current === sessionId ? null : current);
    setPrompts((current) => current.filter((prompt) => prompt.sessionId !== sessionId));
  }

  async function addSession(sessionId: string) {
    const session = (await listSessions()).find((item) => item.sessionId === sessionId);
    if (!session) return;
    const status = latestSessionStatus(session.sessionId);
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
  const connect = (hostId: string) => {
    void run(`host:${hostId}`, async () => {
      await addSession(await connectHost(hostId));
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
    <div className="app">
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
            {section === "hosts" ? <HostsPage hosts={hosts} onConnect={connect} usageByHost={usageByHost} onlineHostIds={onlineHostIds} /> : <div className="empty">{t(titles[section])}</div>}
          </div>
          <Suspense fallback={<div className="empty">{t("Loading terminal…")}</div>}>
            {sessions.map((session) => <SessionPage key={session.sessionId} session={session} visible={activeSessionId === session.sessionId}
              blocked={prompts.length > 0} onReconnect={() => reconnect(session.sessionId)} />)}
          </Suspense>
        </main>
      </div>
      {prompts[0] && <PromptDialog key={prompts[0].promptId} prompt={prompts[0]} onAnswered={(id) => setPrompts((current) => current.filter((prompt) => prompt.promptId !== id))} />}
    </div>
  );
}
