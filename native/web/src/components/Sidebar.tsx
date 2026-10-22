import { t } from "../i18n";
import { Icon, type IconName } from "../icons";

export type Section = "hosts" | "keychain" | "snippets" | "known-hosts" | "logs" | "settings";

const items: { id: Section; label: string; icon: IconName }[] = [
  { id: "hosts", label: "Hosts", icon: "hosts" },
  { id: "keychain", label: "Keychain", icon: "keychain" },
  { id: "snippets", label: "Snippets", icon: "snippets" },
  { id: "known-hosts", label: "Known Hosts", icon: "known" },
  { id: "logs", label: "Logs", icon: "logs" },
];

function NavItem({ id, label, icon, active, onSelect }: { id: Section; label: string; icon: IconName; active: boolean; onSelect: (s: Section) => void }) {
  return (
    <button className={`nav-item${active ? " nav-active" : ""}`} aria-current={active ? "page" : undefined} onClick={() => onSelect(id)}>
      <span className="nav-icon"><Icon name={icon} /></span>
      {t(label)}
    </button>
  );
}

export function Sidebar({ section, onSelect }: { section: Section; onSelect: (s: Section) => void }) {
  return (
    <nav className="sidebar" aria-label={t("Navigation")}>
      <div className="sidebar-brand"><img src="/logo.svg" alt="" /><span>OpenTermius</span></div>
      <p className="sidebar-label">{t("Workspace")}</p>
      {items.slice(0, 3).map((item) => (
        <NavItem key={item.id} {...item} active={section === item.id} onSelect={onSelect} />
      ))}
      <p className="sidebar-label sidebar-label-library">{t("Activity")}</p>
      {items.slice(3).map((item) => (
        <NavItem key={item.id} {...item} active={section === item.id} onSelect={onSelect} />
      ))}
      <div className="sidebar-spacer" />
      <NavItem id="settings" label="Settings" icon="settings" active={section === "settings"} onSelect={onSelect} />
      <div className="sidebar-footer"><Icon name="keychain" size={12} /><span>{t("Stored on this device")}</span></div>
    </nav>
  );
}
