import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { SessionInfo } from "../api/sessions";
import { t } from "../i18n";
import { Icon } from "../icons";

export interface TabsProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  onSelect: (sessionId: string) => void;
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
      {sessions.length > 0 && <div ref={tabsRef} className="session-tabs" role="tablist" aria-label={t("Sessions")} onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key) || (event.target as HTMLElement).getAttribute("role") !== "tab") return;
        event.preventDefault();
        const tabs = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
        const current = tabs.indexOf(event.target as HTMLButtonElement);
        const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
        tabs[next].click();
        tabs[next].focus();
      }}>
        {sessions.map((session, index) => (
          <div className={`session-tab${activeSessionId === session.sessionId ? " tab-active" : ""}${session.connected ? "" : " tab-off"}`} key={session.sessionId}>
            <button className="session-tab-select" role="tab" aria-selected={activeSessionId === session.sessionId}
              tabIndex={activeSessionId === session.sessionId || (activeSessionId === null && index === 0) ? 0 : -1} aria-controls={`session-panel-${session.sessionId}`} title={session.title} onClick={() => onSelect(session.sessionId)}>
              <span className="tab-icon">{session.local ? <Icon name="prompt" size={14} /> : <span className={`dot${session.connected ? " dot-online" : ""}`} role="img" aria-label={t(session.connected ? "Connected" : "Disconnected")} />}</span>
              <span className="session-tab-title">{session.title}</span>
            </button>
            <button className="icon-button tab-action" data-tooltip={t("Session menu")} aria-label={t("Session menu")} aria-expanded={menu?.sessionId === session.sessionId} onClick={(event) => {
              menuTrigger.current = event.currentTarget;
              const rect = event.currentTarget.getBoundingClientRect();
              setMenu(menu?.sessionId === session.sessionId ? null : { sessionId: session.sessionId, left: Math.max(8, Math.min(rect.left, window.innerWidth - 208)), top: rect.bottom + 5 });
            }}><Icon name="more" size={14} /></button>
            <button className="icon-button tab-action tab-close" data-tooltip={t("Close session")} aria-label={t("Close session")} onClick={() => onClose(session.sessionId)}><Icon name="close" size={14} /></button>
          </div>
        ))}
      </div>}
      {sessions.length > 0 && <button className="icon-button new-terminal" data-tooltip={t("Open local terminal")} onClick={onNew} aria-label={t("Open local terminal")}><Icon name="plus" size={16} /></button>}
      {menu && selected && createPortal(
        <div ref={menuRef} className="menu" role="menu" style={{ left: menu.left, top: menu.top }} aria-label={t("Session menu")}>
          <button role="menuitem" onClick={() => { setMenu(null); onReconnect(selected.sessionId); }}><Icon name="refresh" size={16} />{t("Reconnect")}</button>
          <button role="menuitem" onClick={() => { setMenu(null); onDuplicate(selected.sessionId); }}><Icon name="copy" size={16} />{t("Duplicate session")}</button>
          <button role="menuitem" onClick={() => { setMenu(null); onClose(selected.sessionId); }}><Icon name="close" size={16} />{t("Close session")}</button>
        </div>, document.body,
      )}
    </>
  );
}
