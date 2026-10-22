import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { deleteHost, duplicateHost, getHost, listHosts, newHostDraft, parseQuickConnect, renameGroup, type Host, type HostDraft, type Usage } from "../api/hosts";
import { t } from "../i18n";
import { Icon } from "../icons";
import { UsageRing } from "../components/UsageRing";
import { HostEditor } from "../components/HostEditor";
import { HostModal, Menu } from "../components/Menu";

/** Group filter value for hosts without a group (matches the Rust core's NO_GROUP). */
const NO_GROUP = "\u0001";
const UNKNOWN: Usage = { cpu: null, memory: null, disk: null };

function HostCard({ host, usage, online, selected, onSelect, onConnect, onEdit, onMenu }: { host: Host; usage: Usage; online: boolean; selected: boolean; onSelect: () => void; onConnect?: () => void; onEdit: () => void; onMenu: (x: number, y: number) => void }) {
  return (
    <article className={`card${selected ? " card-selected" : ""}`} tabIndex={0} aria-label={host.name} aria-current={selected ? "true" : undefined} onFocus={onSelect} onClick={onSelect} onDoubleClick={onConnect ?? onEdit} onContextMenu={(event) => { event.preventDefault(); onSelect(); onMenu(event.clientX, event.clientY); }}>
      <header className="card-head">
        <span className={`dot${online ? " dot-online" : ""}`} />
        <h3 title={`${host.username}@${host.host}:${host.port}`}>{host.name}</h3>
        <button className="icon-button" disabled={!onConnect} aria-label={t("Connect")} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onConnect?.(); }}><Icon name="terminal" /></button>
        <button className="icon-button" aria-label={t("More")} aria-haspopup="menu" onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); const rect = event.currentTarget.getBoundingClientRect(); onSelect(); onMenu(rect.right - 172, rect.bottom + 4); }}><Icon name="more" /></button>
      </header>
      <div className="card-rings">
        <UsageRing value={usage.cpu} label={t("CPU")} />
        <UsageRing value={usage.memory} label={t("MEM")} />
        <UsageRing value={usage.disk} label={t("DISK")} />
      </div>
    </article>
  );
}

interface Props {
  hosts: Host[];
  onHostsChange?: (hosts: Host[]) => void;
  onConnect?: (hostId: string) => void;
  onQuickConnect?: (draft: HostDraft) => void;
  /** Live usage and connection state from open sessions, by host id. */
  usageByHost?: Record<string, Usage>;
  onlineHostIds?: ReadonlySet<string>;
}

export function HostsPage({ hosts, onHostsChange, onConnect, onQuickConnect, usageByHost = {}, onlineHostIds = new Set<string>() }: Props) {
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
    <button key={value} className={`pill${activeGroup === value ? " pill-active" : ""}`} onClick={() => setGroup(value)} onContextMenu={value && value !== NO_GROUP ? (event) => { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); } : undefined} onKeyDown={(event) => {
      if (value && value !== NO_GROUP && (event.key === "F2" || (event.shiftKey && event.key === "F10"))) { event.preventDefault(); setDialogError(""); setRenaming({ old: value, value }); }
    }}>{label}</button>
  );

  return (
    <section className="page hosts-page" aria-busy={busy}>
      <div className="toolbar">
        <div className="pills glass">
          {pill("", t("All"))}
          {groups.map((g) => pill(g, g))}
          {groups.length > 0 && hasUngrouped && pill(NO_GROUP, t("No group"))}
        </div>
        <div className="toolbar-spacer" />
        <button className="add-button" disabled={busy} aria-label={t("New host")} onClick={() => setEditor(newHostDraft(activeGroup === NO_GROUP ? "" : activeGroup))}><Icon name="plus" /></button>
        <label className="search glass">
          <Icon name="search" />
          <input aria-label={t("Search, or ssh user@host")} value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("Search, or ssh user@host")} onKeyDown={(event) => {
            if (event.key === "Enter" && /^ssh(?:\s|$)/i.test(search.trim())) { event.preventDefault(); quickConnect(); }
            if (event.key === "ArrowDown" && visible.length) { event.preventDefault(); (grid.current?.children[0] as HTMLElement | undefined)?.focus(); }
          }} />
        </label>
      </div>
      {error && <div className="host-error" role="alert">{error}</div>}
      {visible.length === 0 ? (
        <div className="empty">{hosts.length === 0 ? t("No hosts yet") : t("No matching hosts")}</div>
      ) : (
        <div className="grid" ref={grid} onKeyDown={moveSelection}>
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
