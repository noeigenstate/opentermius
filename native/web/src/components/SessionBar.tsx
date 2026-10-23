import type { SessionInfo } from "../api/sessions";
import { t } from "../i18n";
import { Icon } from "../icons";

export function SessionBar({ session, view, onViewChange }: {
  session: SessionInfo;
  view: "terminal" | "sftp";
  onViewChange: (view: "terminal" | "sftp") => void;
}) {
  return (
    <div className="session-bar">
      {session.local ? <span className="chip">{t("Local")}</span> : (
        <div className="segmented" role="group" aria-label={t("Session view")}>
          <button aria-pressed={view === "terminal"} onClick={() => onViewChange("terminal")}><Icon name="terminal" size={14} />{t("Terminal")}</button>
          <button aria-pressed={view === "sftp"} onClick={() => onViewChange("sftp")}><Icon name="folder" size={14} />{t("SFTP")}</button>
        </div>
      )}
      <span className="session-host" title={session.title}>{session.title}</span>
      <span className="session-connection" data-connected={session.connected}><span className={`dot${session.connected ? " dot-online" : ""}`} />{t(session.connected ? "Connected" : "Disconnected")}</span>
    </div>
  );
}
