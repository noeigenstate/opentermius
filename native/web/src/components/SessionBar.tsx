import type { SessionInfo } from "../api/sessions";
import { t } from "../i18n";

export function SessionBar({ session, view, onViewChange }: {
  session: SessionInfo;
  view: "terminal" | "sftp";
  onViewChange: (view: "terminal" | "sftp") => void;
}) {
  return (
    <div className="session-bar glass">
      {session.local ? <span className="local-badge">{t("Local")}</span> : (
        <div className="pills" role="group" aria-label={t("Session view")}>
          <button className={`pill${view === "terminal" ? " pill-active" : ""}`} aria-pressed={view === "terminal"} onClick={() => onViewChange("terminal")}>{t("Terminal")}</button>
          <button className={`pill${view === "sftp" ? " pill-active" : ""}`} aria-pressed={view === "sftp"} onClick={() => onViewChange("sftp")}>{t("SFTP")}</button>
        </div>
      )}
      <span className="session-host" title={session.title}>{session.title}</span>
      <span className="session-connection"><span className={`dot ${session.connected ? "dot-online" : "dot-disconnected"}`} />{t(session.connected ? "Connected" : "Disconnected")}</span>
    </div>
  );
}
