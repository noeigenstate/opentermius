import { useMemo, useState } from "react";
import type { Host, Usage } from "../api/hosts";
import { t } from "../i18n";
import { Icon } from "../icons";
import { UsageRing } from "../components/UsageRing";

/** Group filter value for hosts without a group (matches the Rust core's NO_GROUP). */
const NO_GROUP = "\u0001";
const UNKNOWN: Usage = { cpu: null, memory: null, disk: null };

function HostCard({ host, usage, online, selected, onSelect, onConnect }: { host: Host; usage: Usage; online: boolean; selected: boolean; onSelect: () => void; onConnect: () => void }) {
  return (
    <article className={`card${selected ? " card-selected" : ""}`} onClick={onSelect} onDoubleClick={onConnect}>
      <header className="card-head">
        <span className={`dot${online ? " dot-online" : ""}`} />
        <h3 title={`${host.username}@${host.host}:${host.port}`}>{host.name}</h3>
        <button className="icon-button" aria-label={t("Connect")} onClick={(event) => { event.stopPropagation(); onConnect(); }} onDoubleClick={(event) => event.stopPropagation()}><Icon name="terminal" /></button>
        <button className="icon-button" aria-label={t("More")}><Icon name="more" /></button>
      </header>
      <div className="card-rings">
        <UsageRing value={usage.cpu} label={t("CPU")} />
        <UsageRing value={usage.memory} label={t("MEM")} />
        <UsageRing value={usage.disk} label={t("DISK")} />
      </div>
    </article>
  );
}

export function HostsPage({ hosts, onConnect, usageByHost = {}, onlineHostIds = new Set<string>() }: {
  hosts: Host[];
  onConnect: (hostId: string) => void;
  usageByHost?: Record<string, Usage>;
  onlineHostIds?: ReadonlySet<string>;
}) {
  const [group, setGroup] = useState("");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState("");

  const groups = useMemo(() => [...new Set(hosts.map((h) => h.group).filter(Boolean))].sort(), [hosts]);
  const hasUngrouped = hosts.some((h) => !h.group);
  // A group that disappeared (renamed or emptied) falls back to "All".
  const activeGroup = group === "" || (group === NO_GROUP && hasUngrouped) || groups.includes(group) ? group : "";
  const query = search.trim().toLowerCase();
  const visible = hosts
    .filter((h) => activeGroup === "" || (activeGroup === NO_GROUP ? !h.group : h.group === activeGroup))
    .filter((h) => !query || [h.name, h.host, h.username, h.group].some((v) => v.toLowerCase().includes(query)));

  const pill = (value: string, label: string) => (
    <button key={value} className={`pill${activeGroup === value ? " pill-active" : ""}`} onClick={() => setGroup(value)}>{label}</button>
  );

  return (
    <section className="page">
      <div className="toolbar">
        <div className="pills glass">
          {pill("", t("All"))}
          {groups.map((g) => pill(g, g))}
          {groups.length > 0 && hasUngrouped && pill(NO_GROUP, t("No group"))}
        </div>
        <div className="toolbar-spacer" />
        <button className="add-button" aria-label={t("New host")}><Icon name="plus" /></button>
        <label className="search glass">
          <Icon name="search" />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("Search, or ssh user@host")} />
        </label>
      </div>
      {visible.length === 0 ? (
        <div className="empty">{hosts.length === 0 ? t("No hosts yet") : t("No matching hosts")}</div>
      ) : (
        <div className="grid">
          {visible.map((h) => (
            <HostCard key={h.id} host={h} usage={usageByHost[h.id] ?? UNKNOWN} online={onlineHostIds.has(h.id)} selected={selected === h.id} onSelect={() => setSelected(h.id)} onConnect={() => onConnect(h.id)} />
          ))}
        </div>
      )}
    </section>
  );
}
