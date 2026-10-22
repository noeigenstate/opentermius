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
    <button className={`nav-item${active ? " nav-active" : ""}`} onClick={() => onSelect(id)}>
      <span className="nav-icon"><Icon name={icon} /></span>
      {t(label)}
    </button>
  );
}

export function Sidebar({ section, onSelect }: { section: Section; onSelect: (s: Section) => void }) {
  return (
    <nav className="sidebar">
      {items.map((item) => (
        <NavItem key={item.id} {...item} active={section === item.id} onSelect={onSelect} />
      ))}
      <div className="sidebar-spacer" />
      <NavItem id="settings" label="Settings" icon="settings" active={section === "settings"} onSelect={onSelect} />
    </nav>
  );
}
