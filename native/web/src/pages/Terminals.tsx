import { t } from "../i18n";
import { Icon } from "../icons";

export function TerminalsPage({ onLocalTerminal, onHosts }: { onLocalTerminal: () => void; onHosts: () => void }) {
  return <section className="page">
    <header className="page-header"><div><h1>{t("Terminals")}</h1><p>{t("Open SSH and local terminal sessions.")}</p></div></header>
    <div className="empty-state">
      <span className="empty-glyph"><Icon name="prompt" size={34} /></span>
      <h2>{t("No open terminals")}</h2><p>{t("Connect from Hosts or open a local terminal.")}</p>
      <div className="page-actions"><button className="btn btn-l btn-primary" onClick={onLocalTerminal}><Icon name="prompt" size={16} />{t("Local Terminal")}</button><button className="btn btn-l" onClick={onHosts}>{t("Go to Hosts")}</button></div>
    </div>
  </section>;
}
