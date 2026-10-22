import { t } from "../i18n";

/** Integration placeholder; replaced by the SFTP worker's page. */
export function SftpView({ sessionId }: { sessionId: string }) {
  return <div className="empty" data-session-id={sessionId}>{t("SFTP view is not available yet")}</div>;
}
