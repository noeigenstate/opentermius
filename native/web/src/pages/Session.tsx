import { lazy, Suspense, useState } from "react";
import type { SessionInfo } from "../api/sessions";
import { SessionBar } from "../components/SessionBar";
import { TerminalView } from "../components/TerminalView";
import { t } from "../i18n";

const SftpView = lazy(() => import("./Sftp").then((module) => ({ default: module.SftpView })));

export function SessionPage({ session, visible, blocked, initialView = "terminal", onReconnect }: {
  session: SessionInfo;
  visible: boolean;
  blocked: boolean;
  initialView?: "terminal" | "sftp";
  onReconnect: () => void;
}) {
  const [view, setView] = useState<"terminal" | "sftp">(initialView);
  const [sftpOpened, setSftpOpened] = useState(initialView === "sftp");
  return (
    <section className="session-page" role="tabpanel" id={`session-panel-${session.sessionId}`} hidden={!visible}>
      <SessionBar session={session} view={view} onViewChange={(next) => { setView(next); if (next === "sftp") setSftpOpened(true); }} />
      <TerminalView sessionId={session.sessionId} visible={visible && view === "terminal"} ended={session.ended}
        local={session.local} blocked={blocked} onReconnect={onReconnect} />
      {sftpOpened && <div className="session-sftp" hidden={view !== "sftp"}>
        <Suspense fallback={<div className="empty">{t("Loading SFTP…")}</div>}><SftpView sessionId={session.sessionId} /></Suspense>
      </div>}
    </section>
  );
}
