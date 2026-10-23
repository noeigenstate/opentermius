import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { deleteHost, duplicateHost, getHost, listHosts, newHostDraft, parseQuickConnect, renameGroup, type Host, type HostDraft, type Usage } from "../api/hosts";
import { t } from "../i18n";
import { Icon } from "../icons";
import { UsageRing } from "../components/UsageRing";
import { HostEditor } from "../components/HostEditor";
import { HostModal, Menu } from "../components/Menu";
import "../hosts.css";

/** Group filter value for hosts without a group (matches the Rust core's NO_GROUP). */
const NO_GROUP = "\u0001";
export function filterFavorites(hosts: Host[], favoritesOnly: boolean): Host[] {
  return favoritesOnly ? hosts.filter((host) => host.favorite) : hosts;
}

const UNKNOWN: Usage = { cpu: null, memory: null, disk: null };

function HostCard({ host, usage, online, selected, onSelect, onConnect, onEdit, onMenu }: { host: Host; usage: Usage; online: boolean; selected: boolean; onSelect: () => void; onConnect?: () => void; onEdit: () => void; onMenu: (x: number, y: number) => void }) {
  return (
    <article className={`host-card${selected ? " host-card-selected" : ""}`} tabIndex={0} aria-label={host.name} aria-current={selected ? "true" : undefined} onFocus={onSelect} onClick={onSelect} onDoubleClick={onConnect ?? onEdit} onContextMenu={(event) => { event.preventDefault(); onSelect(); onMenu(event.clientX, event.clientY); }}>
      <header className="host-card-header">
        <span className="host-server-glyph"><Icon name="hosts" size={23} /><i className={`host-indicator${online ? " host-indicator-online" : ""}`} /></span>
        <div className="host-card-identity">
          <h3 title={host.name}>{host.name}</h3>
          <span className={`host-connection-state${online ? " is-connected" : ""}`}>{online ? t("Connected") : t("Disconnected")}</span>
        </div>
        <button className="icon-button host-more" aria-label={t("More")} data-tooltip={t("More")} aria-haspopup="menu" onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); const rect = event.currentTarget.getBoundingClientRect(); onSelect(); onMenu(rect.right - 172, rect.bottom + 4); }}><Icon name="more" /></button>
      </header>
      <p className="host-endpoint" title={`${host.username ? `${host.username}@` : ""}${host.host}:${host.port}`}>
        <span>{host.username && `${host.username}@`}{host.host}</span><span className="host-port-number">:{host.port}</span>
      </p>
      <div className="host-card-labels">
        <span className="host-protocol">{t("SSH")}</span>
        {host.group && <span className="host-tag" title={host.group}>{host.group}</span>}
        {host.tags.slice(0, 2).map((tag) => <span key={tag} className="host-tag" title={tag}>{tag}</span>)}
        {host.tags.length > 2 && <span className="host-tag" data-tooltip={host.tags.slice(2).join(", ")}>+{host.tags.length - 2}</span>}
      </div>
      <div className="host-metrics">
        <UsageRing value={usage.cpu} label={t("CPU")} />
        <UsageRing value={usage.memory} label={t("MEM")} />
        <UsageRing value={usage.disk} label={t("DISK")} />
      </div>
      <footer className="host-card-footer">
        <span className="host-usage-note">{online ? t("Session metrics") : t("No active session")}</span>
        <button className="host-connect" disabled={!onConnect} data-tooltip={t("Open an SSH connection")} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onConnect?.(); }}><Icon name="terminal" size={14} />{t("Connect")}</button>
      </footer>
    </article>
  );
}

interface Props {
  hosts: Host[];
  favoritesOnly?: boolean;
  onHostsChange?: (hosts: Host[]) => void;
  onConnect?: (hostId: string) => void;
  onQuickConnect?: (draft: HostDraft) => void;
  /** Live usage and connection state from open sessions, by host id. */
  usageByHost?: Record<string, Usage>;
  onlineHostIds?: ReadonlySet<string>;
}

export function HostsPage({ hosts: allHosts, favoritesOnly = false, onHostsChange, onConnect, onQuickConnect, usageByHost = {}, onlineHostIds = new Set<string>() }: Props) {
  const hosts = useMemo(() => filterFavorites(allHosts, favoritesOnly), [allHosts, favoritesOnly]);
  const [group, setGroup] = useState("");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState("");
  const [editor, setEditor] = useState<HostDraft | null>(null);
  const [menu, setMenu] = useState<{ host: Host; x: number; y: number } | null>(null);
  const [deleting, setDeleting] = useState<Host | null>(null);
  const [renaming, setRenaming] = useState<{ old: string; value: string } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [dialogError, setDialogError] = useState("");
  const grid = useRef<HTMLDivElement>(null);
  const pending = useRef(false);

  const groups = useMemo(() => [...new Set(hosts.map((h) => h.group).filter(Boolean))].sort(), [hosts]);
  const hasUngrouped = hosts.some((h) => !h.group);
  // A group that disappeared (renamed or emptied) falls back to "All".
  const activeGroup = group === "" || (group === NO_GROUP && hasUngrouped) || groups.includes(group) ? group : "";
  const query = search.trim().toLowerCase();
  const visible = hosts
    .filter((h) => activeGroup === "" || (activeGroup === NO_GROUP ? !h.group : h.group === activeGroup))
    .filter((h) => !query || [h.name, h.host, h.username, h.group, ...(h.tags ?? [])].some((v) => v.toLowerCase().includes(query)));
  const connectedCount = hosts.filter((host) => onlineHostIds.has(host.id)).length;
  const addHost = () => setEditor(newHostDraft(activeGroup === NO_GROUP ? "" : activeGroup));

  const refresh = async () => {
    try { onHostsChange?.(await listHosts()); }
    catch (reason) { setError(String(reason)); }
  };
  const run = async (action: () => Promise<void>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError("");
    try { await action(); } catch (reason) { setError(String(reason)); }
    finally { pending.current = false; setBusy(false); }
  };
  const edit = (hostId: string) => void run(async () => { setEditor(await getHost(hostId)); });
  const askDelete = (host: Host) => { setDialogError(""); setDeleting(host); };
  const duplicate = (hostId: string) => void run(async () => { setSelected(await duplicateHost(hostId)); await refresh(); });
  const quickConnect = () => void run(async () => {
    const draft = await parseQuickConnect(search);
    setSearch("");
    if (onQuickConnect) onQuickConnect(draft); else setEditor(draft);
  });
  const remove = async () => {
    if (!deleting || pending.current) return;
    pending.current = true;
    setBusy(true);
    setDialogError("");
    try { await deleteHost(deleting.id); setDeleting(null); setSelected(""); await refresh(); }
    catch (reason) { setDialogError(String(reason)); }
    finally { pending.current = false; setBusy(false); }
  };
  const rename = async () => {
    if (!renaming || pending.current) return;
    pending.current = true;
    setBusy(true);
    setDialogError("");
    try {
      const name = renaming.value.trim();
      await renameGroup(renaming.old, name);
      if (group === renaming.old) setGroup(name);
      setRenaming(null);
      await refresh();
    } catch (reason) { setDialogError(String(reason)); }
    finally { pending.current = false; setBusy(false); }
  };
  const moveSelection = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLButtonElement || editor || menu || deleting || renaming) return;
    const index = visible.findIndex((host) => host.id === selected);
    if (event.key === "Enter" && index >= 0) { event.preventDefault(); onConnect?.(selected); return; }
    if (event.key === "Delete" && index >= 0) { event.preventDefault(); askDelete(visible[index]); return; }
    const columns = grid.current ? getComputedStyle(grid.current).gridTemplateColumns.split(" ").length : 1;
    const delta = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" ? -columns : event.key === "ArrowDown" ? columns : 0;
    if (!delta || !visible.length) return;
    event.preventDefault();
    const next = index < 0 ? (delta < 0 ? visible.length - 1 : 0) : Math.max(0, Math.min(visible.length - 1, index + delta));
    setSelected(visible[next].id);
    (grid.current?.children[next] as HTMLElement | undefined)?.focus();
  };

  const pill = (value: string, label: string) => (
    <button key={value} className={`pill${activeGroup === value ? " pill-active" : ""}`} aria-pressed={activeGroup === value} onClick={() => setGroup(value)} onContextMenu={value && value !== NO_GROUP ? (event) => { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); } : undefined} onKeyDown={(event) => {
      if (value && value !== NO_GROUP && (event.key === "F2" || (event.shiftKey && event.key === "F10"))) { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); }
    }}>{label}</button>
  );

  return (
    <section className="page hosts-page" aria-busy={busy}>
      <header className="hosts-header">
        <div>
          <h1>{t(favoritesOnly ? "Favorites" : "Hosts")}</h1>
          <p>{t(favoritesOnly ? "Hosts you starred" : "Manage saved connections and open SSH sessions.")}</p>
        </div>
        <button className="host-create" data-depth disabled={busy} onClick={addHost}><Icon name="plus" size={17} />{t("New host")}</button>
      </header>
      <div className="hosts-summary" aria-label={t("Connection overview")}>
        <span><Icon name="hosts" size={14} /><strong>{hosts.length}</strong>{t("Saved hosts")}</span>
        <span><i className={`host-indicator${connectedCount ? " host-indicator-online" : ""}`} /><strong>{connectedCount}</strong>{t("Connected")}</span>
      </div>
      <div className="hosts-controls">
        <div className="pills hosts-groups" role="group" aria-label={t("Filter by group")}>
          {pill("", t("All"))}
          {groups.map((g) => pill(g, g))}
          {groups.length > 0 && hasUngrouped && pill(NO_GROUP, t("No group"))}
        </div>
        <label className="search hosts-search">
          <Icon name="search" />
          <input aria-label={t("Search, or ssh user@host")} value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("Search, or ssh user@host")} onKeyDown={(event) => {
            if (event.key === "Enter" && /^ssh(?:\s|$)/i.test(search.trim())) { event.preventDefault(); quickConnect(); }
            if (event.key === "ArrowDown" && visible.length) { event.preventDefault(); (grid.current?.children[0] as HTMLElement | undefined)?.focus(); }
          }} />
          {search && <button type="button" className="hosts-clear" aria-label={t("Clear search")} data-tooltip={t("Clear search")} onClick={() => setSearch("")}><Icon name="close" size={13} /></button>}
        </label>
      </div>
      {error && <div className="host-error" role="alert">{error}</div>}
      <div className="hosts-section-heading">
        <h2>{activeGroup === NO_GROUP ? t("No group") : activeGroup || t("All hosts")}<span>{visible.length}</span></h2>
        <span>{onConnect ? t("Double-click to connect") : t("Double-click to edit")}</span>
      </div>
      {visible.length === 0 ? (
        <div className="hosts-empty">
          <span className="hosts-empty-glyph"><Icon name={hosts.length === 0 ? "hosts" : "search"} size={34} /></span>
          <h2>{hosts.length === 0 ? t(favoritesOnly ? "No favorites yet" : "No hosts yet") : t("No matching hosts")}</h2>
          <p>{hosts.length === 0 ? t(favoritesOnly ? "Star a host to find it here." : "Add a host to keep its connection details in one place.") : t("Try another search or choose a different group.")}</p>
          <button className={hosts.length === 0 ? "host-create" : "host-empty-reset"} data-depth disabled={busy} onClick={hosts.length === 0 ? addHost : () => { setSearch(""); setGroup(""); }}><Icon name={hosts.length === 0 ? "plus" : "close"} size={16} />{hosts.length === 0 ? t("New host") : t("Clear filters")}</button>
        </div>
      ) : (
        <div className="hosts-grid" ref={grid} onKeyDown={moveSelection}>
          {visible.map((h) => (
            <HostCard key={h.id} host={h} usage={usageByHost[h.id] ?? UNKNOWN} online={onlineHostIds.has(h.id)} selected={selected === h.id} onSelect={() => setSelected(h.id)} onConnect={onConnect ? () => onConnect(h.id) : undefined} onEdit={() => edit(h.id)} onMenu={(x, y) => setMenu({ host: h, x, y })} />
          ))}
        </div>
      )}
      {menu && <Menu x={menu.x} y={menu.y} onClose={() => setMenu(null)} items={[
        { label: t("Edit"), onSelect: () => edit(menu.host.id) },
        { label: t("Duplicate"), onSelect: () => duplicate(menu.host.id) },
        { label: t("Delete"), danger: true, onSelect: () => askDelete(menu.host) },
        { label: t("Connect"), disabled: !onConnect, onSelect: () => onConnect?.(menu.host.id) },
      ]} />}
      {editor && <HostEditor initial={editor} groups={groups} onClose={() => setEditor(null)} onSaved={async (hostId) => { setSelected(hostId); await refresh(); }} onConnect={onConnect} />}
      {deleting && <HostModal titleId="host-delete-title" onClose={() => { if (!busy) setDeleting(null); }}>
        <h2 id="host-delete-title">{t("Delete host?")}</h2><p>{t('Remove "{name}"? This cannot be undone.', { name: deleting.name })}</p>
        {dialogError && <div className="host-error" role="alert">{dialogError}</div>}
        <div className="host-dialog-actions"><button autoFocus className="host-button" disabled={busy} onClick={() => setDeleting(null)}>{t("Cancel")}</button><button className="host-button host-danger-button" disabled={busy} onClick={() => void remove()}>{t("Delete")}</button></div>
      </HostModal>}
      {renaming && <HostModal titleId="host-rename-title" onClose={() => { if (!busy) setRenaming(null); }}>
        <form onSubmit={(event) => { event.preventDefault(); void rename(); }}>
          <h2 id="host-rename-title">{t("Rename group")}</h2><p>{t("Rename the group and move its hosts with it.")}</p>
          <label className="host-field">{t("Group name")}<input autoFocus value={renaming.value} disabled={busy} onChange={(event) => setRenaming({ ...renaming, value: event.target.value })} required /></label>
          {dialogError && <div className="host-error" role="alert">{dialogError}</div>}
          <div className="host-dialog-actions"><button type="button" className="host-button" disabled={busy} onClick={() => setRenaming(null)}>{t("Cancel")}</button><button type="submit" className="host-button host-primary" disabled={busy || !renaming.value.trim()}>{t("Rename")}</button></div>
        </form>
      </HostModal>}
    </section>
  );
}
