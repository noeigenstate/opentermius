import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { SessionInfo } from "../api/sessions";
import { t } from "../i18n";

export interface TabsProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  onSelect: (sessionId: string | null) => void;
  onNew: () => void;
  onReconnect: (sessionId: string) => void;
  onDuplicate: (sessionId: string) => void;
  onClose: (sessionId: string) => void;
}

export function Tabs({ sessions, activeSessionId, onSelect, onNew, onReconnect, onDuplicate, onClose }: TabsProps) {
  const [menu, setMenu] = useState<{ sessionId: string; left: number; top: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  const menuTrigger = useRef<HTMLButtonElement | null>(null);
  const selected = sessions.find((session) => session.sessionId === menu?.sessionId);
  useEffect(() => {
    tabsRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeSessionId]);
  useEffect(() => {
    if (!menu) return;
    menuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    function dismiss(event: PointerEvent) {
      if (!menuRef.current?.contains(event.target as Node) && !menuTrigger.current?.contains(event.target as Node)) setMenu(null);
    }
    function escape(event: KeyboardEvent) {
      if (event.key === "Escape") { setMenu(null); menuTrigger.current?.focus(); }
    }
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [menu]);

  return (
    <>
      <div ref={tabsRef} className="session-tabs" role="tablist" aria-label={t("Sessions")} onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key) || (event.target as HTMLElement).getAttribute("role") !== "tab") return;
        event.preventDefault();
        const tabs = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
        const current = tabs.indexOf(event.target as HTMLButtonElement);
        const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
        tabs[next].click();
        tabs[next].focus();
      }}>
        <button className={`tab${activeSessionId === null ? " tab-active" : ""}`} role="tab" aria-selected={activeSessionId === null}
          tabIndex={activeSessionId === null ? 0 : -1} aria-controls="hosts-panel" onClick={() => onSelect(null)}>
          <img src="/logo.svg" alt="" width={18} height={18} /><span>{t("Hosts")}</span>
        </button>
        {sessions.map((session) => (
          <div className={`tab session-tab${activeSessionId === session.sessionId ? " tab-active" : ""}`} key={session.sessionId}>
            <button className="session-tab-select" role="tab" aria-selected={activeSessionId === session.sessionId}
              tabIndex={activeSessionId === session.sessionId ? 0 : -1} aria-controls={`session-panel-${session.sessionId}`} title={session.title} onClick={() => onSelect(session.sessionId)}>
              <span className={`dot ${session.connected ? "dot-online" : "dot-disconnected"}`} aria-label={t(session.connected ? "Connected" : "Disconnected")} />
              <span className="session-tab-title">{session.title}</span>
            </button>
            <button className="tab-action" aria-label={t("Session menu")} aria-expanded={menu?.sessionId === session.sessionId} onClick={(event) => {
              menuTrigger.current = event.currentTarget;
              const rect = event.currentTarget.getBoundingClientRect();
              setMenu(menu?.sessionId === session.sessionId ? null : { sessionId: session.sessionId, left: Math.max(8, Math.min(rect.left, window.innerWidth - 208)), top: rect.bottom + 5 });
            }}>⋯</button>
            <button className="tab-action tab-close" aria-label={t("Close session")} onClick={() => onClose(session.sessionId)}>×</button>
          </div>
        ))}
      </div>
      <button className="new-terminal" onClick={onNew} aria-label={t("Open local terminal")} title={t("Open local terminal")}>+</button>
      {menu && selected && createPortal(
        <div ref={menuRef} className="session-menu" style={{ left: menu.left, top: menu.top }} aria-label={t("Session menu")}>
          <button onClick={() => { setMenu(null); onReconnect(selected.sessionId); }}>{t("Reconnect")}</button>
          <button onClick={() => { setMenu(null); onDuplicate(selected.sessionId); }}>{t("Duplicate session")}</button>
          <button onClick={() => { setMenu(null); onClose(selected.sessionId); }}>{t("Close session")}</button>
        </div>, document.body,
      )}
    </>
  );
}
