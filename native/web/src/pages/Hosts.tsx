import { useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { deleteHost, duplicateHost, getHost, listHosts, newHostDraft, parseQuickConnect, renameGroup, setHostFavorite, type Host, type HostDraft, type Usage } from "../api/hosts";
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

export function groupCounts(hosts: Host[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const host of hosts) counts.set(host.group, (counts.get(host.group) ?? 0) + 1);
  return counts;
}

export function hostTone(id: string) {
  const tones = ["blue", "purple", "green", "orange", "pink", "cyan"] as const;
  let hash = 0;
  for (let index = 0; index < id.length; index++) hash = (Math.imul(hash, 31) + id.charCodeAt(index)) >>> 0;
  return tones[hash % tones.length];
}

const UNKNOWN: Usage = { cpu: null, memory: null, disk: null };

function HostCard({ host, usage, online, selected, busy, onSelect, onConnect, onFavorite, onEdit, onMenu }: { host: Host; usage: Usage; online: boolean; selected: boolean; busy: boolean; onSelect: () => void; onConnect?: () => void; onFavorite: () => void; onEdit: () => void; onMenu: (x: number, y: number) => void }) {
  const favoriteLabel = t(host.favorite ? "Remove from favorites" : "Add to favorites");
  const openMenu = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    const rect = event.currentTarget.getBoundingClientRect();
    onSelect();
    onMenu(rect.right - 172, rect.bottom + 4);
  };
  return (
    <article className={`host-card${selected ? " host-card-selected" : ""}`} data-online={online} data-selected={selected} tabIndex={0} aria-label={host.name} aria-current={selected ? "true" : undefined} onFocus={onSelect} onClick={onSelect} onDoubleClick={onConnect ?? onEdit} onContextMenu={(event) => { event.preventDefault(); onSelect(); onMenu(event.clientX, event.clientY); }}>
      <header className="host-card-header">
        <span className="host-server-glyph" data-tone={hostTone(host.id)}><Icon name="hosts" size={22} /></span>
        <div className="host-card-identity">
          <h3 title={host.name}>{host.name}</h3>
          <span className={`host-connection-state${online ? " is-connected" : ""}`}><i className={`dot${online ? " dot-online" : ""}`} />{online ? t("SSH connected") : t("SSH not connected")}</span>
          <p className="host-endpoint" title={`${host.username ? `${host.username}@` : ""}${host.host}:${host.port}`}>
            <span>{host.username && `${host.username}@`}{host.host}</span><span className="host-port-number">:{host.port}</span>
          </p>
        </div>
        <button className="icon-button icon-button-sm host-favorite" aria-pressed={host.favorite} aria-label={favoriteLabel} data-tooltip={favoriteLabel} disabled={busy} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onFavorite(); }}><Icon name="star" size={16} /></button>
        <button className="icon-button icon-button-sm host-more" aria-label={t("More")} data-tooltip={t("More")} aria-haspopup="menu" onDoubleClick={(event) => event.stopPropagation()} onClick={openMenu}><Icon name="more" size={16} /></button>
      </header>
      <div className="host-card-labels">
        <span className="chip">{t("SSH")}</span>
        {host.group && <span className="chip" title={host.group}>{host.group}</span>}
        {host.tags.slice(0, 2).map((tag) => <span key={tag} className="chip" title={tag}>{tag}</span>)}
        {host.tags.length > 2 && <span className="chip" data-tooltip={host.tags.slice(2).join(", ")}>+{host.tags.length - 2}</span>}
      </div>
      <div className="host-metrics">
        <UsageRing value={usage.cpu} label={t("CPU")} />
        <UsageRing value={usage.memory} label={t("Memory")} />
        <UsageRing value={usage.disk} label={t("Disk")} />
      </div>
      <div className="host-card-actions">
        <button className="btn btn-l btn-tonal host-connect" disabled={!onConnect} data-tooltip={t("Open an SSH connection")} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onConnect?.(); }}><Icon name="prompt" size={18} />{t("Connect")}</button>
      </div>
    </article>
  );
}

interface Props {
  hosts: Host[];
  favoritesOnly?: boolean;
  onHostsChange?: (hosts: Host[]) => void;
  onConnect?: (hostId: string) => void;
  onSftp?: (hostId: string) => void;
  onLocalTerminal?: () => void;
  onQuickConnect?: (draft: HostDraft) => void;
  /** Live usage and connection state from open sessions, by host id. */
  usageByHost?: Record<string, Usage>;
  onlineHostIds?: ReadonlySet<string>;
}

export function HostsPage({ hosts: allHosts, favoritesOnly = false, onHostsChange, onConnect, onSftp, onLocalTerminal, onQuickConnect, usageByHost = {}, onlineHostIds = new Set<string>() }: Props) {
  const hosts = useMemo(() => filterFavorites(allHosts, favoritesOnly), [allHosts, favoritesOnly]);
  const viewKey = favoritesOnly ? "favorites-view" : "hosts-view";
  const [view, setView] = useState<"grid" | "list">(() => {
    try { return localStorage.getItem(viewKey) === "list" ? "list" : "grid"; }
    catch { return "grid"; }
  });
  const changeView = (next: "grid" | "list") => {
    setView(next);
    try { localStorage.setItem(viewKey, next); } catch { /* The view still works when storage is unavailable. */ }
  };
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

  const counts = useMemo(() => groupCounts(hosts), [hosts]);
  const groups = useMemo(() => [...counts.keys()].filter(Boolean).sort(), [counts]);
  const hasUngrouped = counts.has("");
  // A group that disappeared (renamed or emptied) falls back to "All".
  const activeGroup = group === "" || (group === NO_GROUP && hasUngrouped) || groups.includes(group) ? group : "";
  const query = search.trim().toLowerCase();
  const visible = hosts
    .filter((h) => activeGroup === "" || (activeGroup === NO_GROUP ? !h.group : h.group === activeGroup))
    .filter((h) => !query || [h.name, h.host, h.username, h.group, ...(h.tags ?? [])].some((v) => v.toLowerCase().includes(query)));
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
  const toggleFavorite = (host: Host) => void run(async () => { await setHostFavorite(host.id, !host.favorite); await refresh(); });
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
    const columns = view === "list" || !grid.current ? 1 : getComputedStyle(grid.current).gridTemplateColumns.split(" ").length;
    const delta = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" ? -columns : event.key === "ArrowDown" ? columns : 0;
    if (!delta || !visible.length) return;
    event.preventDefault();
    const next = index < 0 ? (delta < 0 ? visible.length - 1 : 0) : Math.max(0, Math.min(visible.length - 1, index + delta));
    setSelected(visible[next].id);
    (grid.current?.children[next] as HTMLElement | undefined)?.focus();
  };

  const pill = (value: string, label: string, count: number) => (
    <button key={value} className="pill" aria-pressed={activeGroup === value} onClick={() => setGroup(value)} onContextMenu={value && value !== NO_GROUP ? (event) => { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); } : undefined} onKeyDown={(event) => {
      if (value && value !== NO_GROUP && (event.key === "F2" || (event.shiftKey && event.key === "F10"))) { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); }
    }}><span className="pill-label host-group-label" data-text={label}>{label}</span><span className="pill-count host-group-count">{count}</span></button>
  );

  return (
    <section className="page hosts-page" aria-busy={busy}>
      <header className="page-header">
        <div>
          <h1>{t(favoritesOnly ? "Favorites" : "Hosts")}</h1>
          <p>{t(favoritesOnly ? "Hosts you starred" : "Manage saved connections and open SSH sessions.")}</p>
        </div>
        <div className="page-actions">{!favoritesOnly && onLocalTerminal && <button className="btn btn-l host-local-terminal" onClick={onLocalTerminal}><Icon name="prompt" size={16} />{t("Local Terminal")}</button>}<button className="btn btn-primary btn-l" disabled={busy} onClick={addHost}><Icon name="plus" size={16} />{t("New host")}</button></div>
      </header>
      <div className="page-controls">
        <div className="pills hosts-groups" role="group" aria-label={t("Filter by group")}>
          {pill("", t("All"), hosts.length)}
          {groups.map((g) => pill(g, g, counts.get(g) ?? 0))}
          {hasUngrouped && pill(NO_GROUP, t("No group"), counts.get("") ?? 0)}
        </div>
        <div className="page-controls-end">
          <label className="search hosts-search">
            <Icon name="search" size={18} />
            <input aria-label={t("Search hosts, IPs or tags…")} value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("Search hosts, IPs or tags…")} onKeyDown={(event) => {
              if (event.key === "Enter" && /^ssh(?:\s|$)/i.test(search.trim())) { event.preventDefault(); quickConnect(); }
              if (event.key === "ArrowDown" && visible.length) { event.preventDefault(); (grid.current?.children[0] as HTMLElement | undefined)?.focus(); }
            }} />
            {search && <button type="button" className="icon-button icon-button-sm hosts-clear" aria-label={t("Clear search")} data-tooltip={t("Clear search")} onClick={() => setSearch("")}><Icon name="close" size={14} /></button>}
          </label>
          <div className="hosts-view-toggle" role="group" aria-label={t("Host view")}>
            <button className="icon-button icon-button-lg icon-button-boxed" aria-label={t("Grid view")} data-tooltip={t("Grid view")} aria-pressed={view === "grid"} onClick={() => changeView("grid")}><Icon name="grid" size={19} /></button>
            <button className="icon-button icon-button-lg icon-button-boxed" aria-label={t("List view")} data-tooltip={t("List view")} aria-pressed={view === "list"} onClick={() => changeView("list")}><Icon name="list" size={19} /></button>
          </div>
        </div>
      </div>
      {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
      {visible.length === 0 ? (
        <div className="empty-state">
          <span className="empty-glyph"><Icon name={hosts.length === 0 ? "hosts" : "search"} size={34} /></span>
          <h2>{hosts.length === 0 ? t(favoritesOnly ? "No favorites yet" : "No hosts yet") : t("No matching hosts")}</h2>
          <p>{hosts.length === 0 ? t(favoritesOnly ? "Star a host to find it here." : "Add a host to keep its connection details in one place.") : t("Try another search or choose a different group.")}</p>
          {(!favoritesOnly || hosts.length > 0) && <button className={`btn btn-l${hosts.length === 0 ? " btn-primary" : ""}`} disabled={busy} onClick={hosts.length === 0 ? addHost : () => { setSearch(""); setGroup(""); }}><Icon name={hosts.length === 0 ? "plus" : "close"} size={16} />{hosts.length === 0 ? t("New host") : t("Clear filters")}</button>}
        </div>
      ) : (
        <div className={`hosts-${view}`} ref={grid} onKeyDown={moveSelection}>
          {visible.map((h) => (
            <HostCard key={h.id} host={h} usage={usageByHost[h.id] ?? UNKNOWN} online={onlineHostIds.has(h.id)} selected={selected === h.id} busy={busy} onSelect={() => setSelected(h.id)} onConnect={onConnect ? () => onConnect(h.id) : undefined} onFavorite={() => toggleFavorite(h)} onEdit={() => edit(h.id)} onMenu={(x, y) => setMenu({ host: h, x, y })} />
          ))}
        </div>
      )}
      {menu && <Menu x={menu.x} y={menu.y} onClose={() => setMenu(null)} items={[
        { label: t("Edit"), icon: "edit", onSelect: () => edit(menu.host.id) },
        { label: t("Duplicate"), icon: "copy", onSelect: () => duplicate(menu.host.id) },
        { label: t("Delete"), icon: "trash", danger: true, onSelect: () => askDelete(menu.host) },
        { label: t("Connect"), icon: "prompt", disabled: !onConnect, onSelect: () => onConnect?.(menu.host.id) },
        { label: t("Open SFTP"), icon: "folder", disabled: !onSftp, onSelect: () => onSftp?.(menu.host.id) },
      ]} />}
      {editor && <HostEditor initial={editor} groups={groups} onClose={() => setEditor(null)} onSaved={async (hostId) => { setSelected(hostId); await refresh(); }} onConnect={onConnect} />}
      {deleting && <HostModal titleId="host-delete-title" onClose={() => { if (!busy) setDeleting(null); }}>
        <h2 id="host-delete-title">{t("Delete host?")}</h2><p>{t('Remove "{name}"? This cannot be undone.', { name: deleting.name })}</p>
        {dialogError && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{dialogError}</span></div>}
        <div className="dialog-actions"><button autoFocus className="btn" disabled={busy} onClick={() => setDeleting(null)}>{t("Cancel")}</button><button className="btn btn-danger" disabled={busy} onClick={() => void remove()}>{t("Delete")}</button></div>
      </HostModal>}
      {renaming && <HostModal titleId="host-rename-title" onClose={() => { if (!busy) setRenaming(null); }}>
        <form onSubmit={(event) => { event.preventDefault(); void rename(); }}>
          <h2 id="host-rename-title">{t("Rename group")}</h2><p>{t("Rename the group and move its hosts with it.")}</p>
          <label className="field">{t("Group name")}<input autoFocus value={renaming.value} disabled={busy} onChange={(event) => setRenaming({ ...renaming, value: event.target.value })} required /></label>
          {dialogError && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{dialogError}</span></div>}
          <div className="dialog-actions"><button type="button" className="btn" disabled={busy} onClick={() => setRenaming(null)}>{t("Cancel")}</button><button type="submit" className="btn btn-primary" disabled={busy || !renaming.value.trim()}>{t("Rename")}</button></div>
        </form>
      </HostModal>}
    </section>
  );
}
