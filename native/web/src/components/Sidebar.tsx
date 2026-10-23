import { t } from "../i18n";
import { Icon, type IconName } from "../icons";

export type Section = "hosts" | "keychain" | "snippets" | "logs" | "favorites" | "settings";

const items: { id: Section; label: string; icon: IconName }[] = [
  { id: "hosts", label: "Hosts", icon: "hosts" },
  { id: "keychain", label: "Keychain", icon: "keychain" },
  { id: "snippets", label: "Snippets", icon: "snippets" },
  { id: "logs", label: "Logs", icon: "logs" },
  { id: "favorites", label: "Favorites", icon: "bookmark" },
  { id: "settings", label: "Settings", icon: "settings" },
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
      <div className="sidebar-brand"><img src="/logo.svg" alt="" /><span>{t("OpenTermius")}</span></div>
      {items.slice(0, 3).map((item) => (
        <NavItem key={item.id} {...item} active={section === item.id} onSelect={onSelect} />
      ))}
      <hr className="sidebar-divider" />
      {items.slice(3).map((item) => (
        <NavItem key={item.id} {...item} active={section === item.id} onSelect={onSelect} />
      ))}
    </nav>
  );
}
