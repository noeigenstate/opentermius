import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";
import { deleteHost, duplicateHost, getHost, listHosts, newHostDraft, parseQuickConnect, renameGroup, reorderHosts, setHostFavorite, type Host, type HostDraft, type Usage } from "../api/hosts";
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

/** Move beside the visible neighbour; hidden hosts keep their relative order. */
export function moveVisibleHost(hosts: Host[], visibleIds: string[], id: string, target: number): Host[] {
  const from = visibleIds.indexOf(id);
  const to = Math.max(0, Math.min(visibleIds.length - 1, target));
  if (from < 0 || from === to) return hosts;
  const moving = hosts.find((host) => host.id === id);
  const neighbour = visibleIds[to];
  if (!moving || !hosts.some((host) => host.id === neighbour)) return hosts;
  const next = hosts.filter((host) => host.id !== id);
  const index = next.findIndex((host) => host.id === neighbour);
  next.splice(index + (to > from ? 1 : 0), 0, moving);
  return next;
}

interface HostDrag {
  pointerId: number;
  card: HTMLElement;
  cards: HTMLElement[];
  rects: DOMRect[];
  ids: string[];
  from: number;
  to: number;
  x: number;
  y: number;
  active: boolean;
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

// Official brand colours, from the same Simple Icons data as the glyphs.
const OS_ICONS = {
  ubuntu: { name: "osUbuntu", label: "Ubuntu", color: "#E95420" },
  debian: { name: "osDebian", label: "Debian", color: "#A81D33" },
  centos: { name: "osCentos", label: "CentOS", color: "#262577" },
  rhel: { name: "osRhel", label: "Red Hat Enterprise Linux", color: "#EE0000" },
  fedora: { name: "osFedora", label: "Fedora", color: "#51A2DA" },
  rocky: { name: "osRocky", label: "Rocky Linux", color: "#10B981" },
  almalinux: { name: "osAlmalinux", label: "AlmaLinux", color: "#000000" },
  alpine: { name: "osAlpine", label: "Alpine Linux", color: "#0D597F" },
  arch: { name: "osArch", label: "Arch Linux", color: "#1793D1" },
  opensuse: { name: "osOpensuse", label: "openSUSE", color: "#73BA25" },
  amazon: { name: "osAmazon", label: "Amazon Linux", color: "#FF9900" },
  kali: { name: "osKali", label: "Kali Linux", color: "#557C94" },
  raspbian: { name: "osRaspbian", label: "Raspbian", color: "#A22846" },
  linux: { name: "osLinux", label: "Linux", color: "#FCC624" },
  freebsd: { name: "osFreebsd", label: "FreeBSD", color: "#AB2B28" },
  macos: { name: "osMacos", label: "macOS", color: "#000000" },
  windows: { name: "osWindows", label: "Windows", color: "#0078D4" },
} as const;

export function hostOsIcon(os?: string | null) {
  return os && Object.hasOwn(OS_ICONS, os) ? OS_ICONS[os as keyof typeof OS_ICONS] : { name: "hosts", label: "Server" } as const;
}

/** `focusVisible` works in WebView2 (Chromium) but is not yet in the TypeScript DOM types. */
const focusCard = (card: HTMLElement, visible: boolean) => card.focus({ preventScroll: true, focusVisible: visible } as FocusOptions);

const UNKNOWN: Usage = { cpu: null, memory: null, disk: null };

function HostCard({ host, usage, connectionState, selected, busy, onSelect, onConnect, onFavorite, onEdit, onMenu }: { host: Host; usage: Usage; connectionState?: "connected" | "lost"; selected: boolean; busy: boolean; onSelect: () => void; onConnect?: () => void; onFavorite: () => void; onEdit: () => void; onMenu: (x: number, y: number) => void }) {
  const online = connectionState === "connected";
  const osIcon = hostOsIcon(host.os);
  const osLabel = t(osIcon.label);
  const favoriteLabel = t(host.favorite ? "Remove from favorites" : "Add to favorites");
  const openMenu = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    const rect = event.currentTarget.getBoundingClientRect();
    onSelect();
    onMenu(rect.right - 172, rect.bottom + 4);
  };
  return (
    <article className={`host-card${selected ? " host-card-selected" : ""}`} data-host-id={host.id} data-online={online} data-selected={selected} tabIndex={0} aria-label={host.name} aria-description={t("Drag or press Alt+Arrow to reorder")} aria-current={selected ? "true" : undefined} onFocus={onSelect} onClick={onSelect} onDoubleClick={onConnect ?? onEdit} onContextMenu={(event) => { event.preventDefault(); onSelect(); onMenu(event.clientX, event.clientY); }}>
      <header className="host-card-header">
        <span className="host-server-glyph" data-tone={hostTone(host.id)} style={"color" in osIcon ? { "--brand": osIcon.color } as CSSProperties : undefined} role="img" aria-label={osLabel} data-tooltip={osLabel}><Icon name={osIcon.name} size={22} /></span>
        <div className="host-card-identity">
          <h3 title={host.name}>{host.name}</h3>
          <span className={`host-connection-state${online ? " is-connected" : ""}`}><i className={`dot${online ? " dot-online" : connectionState === "lost" ? " dot-lost" : ""}`} />{online ? t("SSH connected") : connectionState === "lost" ? t("Connection lost") : t("SSH not connected")}</span>
          <p className="host-endpoint" title={`${host.username ? `${host.username}@` : ""}${host.host}:${host.port}`}>
            <span>{host.username && `${host.username}@`}{host.host}</span><span className="host-port-number">:{host.port}</span>
          </p>
        </div>
        <button className="icon-button icon-button-sm host-favorite" aria-pressed={host.favorite} aria-label={favoriteLabel} data-tooltip={favoriteLabel} disabled={busy} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onFavorite(); }}><Icon name="star" size={16} /></button>
        <button className="icon-button icon-button-sm host-more" aria-label={t("More")} data-tooltip={t("More")} aria-haspopup="menu" onDoubleClick={(event) => event.stopPropagation()} onClick={openMenu}><Icon name="more" size={16} /></button>
      </header>
      <div className="host-card-labels">
        {host.tags.slice(0, 2).map((tag) => <span key={tag} className="chip" title={tag}>{tag}</span>)}
        {host.tags.length > 2 && <span className="chip" data-tooltip={host.tags.slice(2).join(", ")}>+{host.tags.length - 2}</span>}
      </div>
      <div className="host-metrics">
        <UsageRing value={online ? usage.cpu : null} label={t("CPU")} />
        <UsageRing value={online ? usage.memory : null} label={t("Memory")} />
        <UsageRing value={online ? usage.disk : null} label={t("Disk")} />
      </div>
      <div className="host-card-actions">
        <button className="btn btn-l btn-tonal host-connect" disabled={!onConnect} data-tooltip={t(online ? "Switch to the open SSH session" : "Open an SSH connection")} onDoubleClick={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onConnect?.(); }}><Icon name="prompt" size={18} />{t(online ? "Open terminal" : "Connect")}</button>
      </div>
    </article>
  );
}

interface Props {
  hosts: Host[];
  favoritesOnly?: boolean;
  onHostsChange?: (hosts: Host[]) => void;
  onConnect?: (hostId: string) => void;
  /** Shows a connected host's open session; the card's main action uses it instead of a second connection. */
  onOpenSession?: (hostId: string) => void;
  onSftp?: (hostId: string) => void;
  onLocalTerminal?: () => void;
  onQuickConnect?: (draft: HostDraft) => void;
  /** Live usage and connection state from open sessions, by host id. */
  usageByHost?: Record<string, Usage>;
  connectionStates?: ReadonlyMap<string, "connected" | "lost">;
}

export function HostsPage({ hosts: allHosts, favoritesOnly = false, onHostsChange, onConnect, onOpenSession, onSftp, onLocalTerminal, onQuickConnect, usageByHost = {}, connectionStates = new Map<string, "connected" | "lost">() }: Props) {
  const hosts = useMemo(() => filterFavorites(allHosts, favoritesOnly), [allHosts, favoritesOnly]);
  const openHost = (hostId: string) => (connectionStates.get(hostId) === "connected" && onOpenSession ? onOpenSession : onConnect)?.(hostId);
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
  const reorderFocus = useRef("");
  // Keyboard moves keep the focus ring; pointer drags restore focus without it.
  const reorderFocusVisible = useRef(false);
  const drag = useRef<HostDrag | null>(null);
  const suppressClick = useRef(false);
  const suppressDoubleClickUntil = useRef(0);

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
  const reorder = (id: string, ids: string[], target: number, keyboard = false) => {
    if (pending.current) return;
    reorderFocusVisible.current = keyboard;
    const next = moveVisibleHost(allHosts, ids, id, target);
    if (next === allHosts) return;
    void run(async () => {
      reorderFocus.current = id;
      onHostsChange?.(next);
      try {
        await reorderHosts(next.map((host) => host.id));
        reorderFocus.current = id;
        onHostsChange?.(await listHosts());
      } catch (reason) {
        reorderFocus.current = id;
        onHostsChange?.(allHosts);
        throw reason;
      }
    });
  };

  const cancelDrag = () => {
    const current = drag.current;
    if (!current) return;
    drag.current = null;
    for (const card of current.cards) {
      card.style.removeProperty("transform");
      delete card.dataset.dragging;
    }
    delete grid.current?.dataset.reordering;
    if (current.card.hasPointerCapture(current.pointerId)) current.card.releasePointerCapture(current.pointerId);
  };
  useEffect(() => {
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && drag.current) { event.preventDefault(); cancelDrag(); }
    };
    const hidden = () => { if (document.hidden) cancelDrag(); };
    window.addEventListener("keydown", escape, true);
    window.addEventListener("blur", cancelDrag);
    window.addEventListener("resize", cancelDrag);
    // Frozen card rects stop being valid when their container scrolls.
    window.addEventListener("scroll", cancelDrag, true);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      cancelDrag();
      window.removeEventListener("keydown", escape, true);
      window.removeEventListener("blur", cancelDrag);
      window.removeEventListener("resize", cancelDrag);
      window.removeEventListener("scroll", cancelDrag, true);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, []);
  useEffect(() => { cancelDrag(); }, [allHosts, activeGroup, query, view, favoritesOnly]);
  useEffect(() => {
    const card = Array.from(grid.current?.children ?? []).find((item) => (item as HTMLElement).dataset.hostId === reorderFocus.current);
    if (card) focusCard(card as HTMLElement, reorderFocusVisible.current);
    reorderFocus.current = "";
  }, [allHosts]);

  const startDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) suppressClick.current = false;
    if (event.button !== 0 || !event.isPrimary || pending.current || drag.current || editor || menu || deleting || renaming) return;
    const target = event.target as HTMLElement;
    if (target.closest("button, a, input, select, textarea, [role='button'], [role='menu'], [contenteditable='true']")) return;
    const card = target.closest<HTMLElement>(".host-card");
    if (!card || !grid.current) return;
    const cards = Array.from(grid.current.children) as HTMLElement[];
    const from = cards.indexOf(card);
    suppressClick.current = false;
    card.setPointerCapture(event.pointerId);
    drag.current = {
      pointerId: event.pointerId, card, cards, rects: [], ids: visible.map((host) => host.id),
      from, to: from, x: event.clientX, y: event.clientY, active: false,
    };
  };
  const moveDrag = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointerId !== event.pointerId) return;
    if (!(event.buttons & 1)) { cancelDrag(); return; }
    const dx = view === "list" ? 0 : event.clientX - current.x;
    const dy = event.clientY - current.y;
    if (!current.active) {
      if (Math.hypot(event.clientX - current.x, dy) <= 5) return;
      current.active = true;
      suppressClick.current = true;
      suppressDoubleClickUntil.current = Date.now() + 600;
      grid.current!.dataset.reordering = "true";
      // Read geometry once, after disabling hover lifts and entrance animation.
      current.cards.forEach((card) => { card.style.transition = "none"; });
      current.rects = current.cards.map((card) => card.getBoundingClientRect());
      current.cards.forEach((card) => { card.style.removeProperty("transition"); });
      current.card.dataset.dragging = "true";
    }
    event.preventDefault();
    const origin = current.rects[current.from];
    const x = origin.left + origin.width / 2 + dx;
    const y = origin.top + origin.height / 2 + dy;
    let to = current.from;
    let distance = Infinity;
    current.rects.forEach((rect, index) => {
      const candidate = Math.hypot(x - rect.left - rect.width / 2, y - rect.top - rect.height / 2);
      if (candidate < distance) { distance = candidate; to = index; }
    });
    if (to !== current.to) {
      current.to = to;
      const order = current.cards.filter((card) => card !== current.card);
      order.splice(to, 0, current.card);
      order.forEach((card, index) => {
        if (card === current.card) return;
        const before = current.rects[current.cards.indexOf(card)];
        const after = current.rects[index];
        card.style.transform = `translate(${after.left - before.left}px, ${after.top - before.top}px)`;
      });
    }
    current.card.style.transform = `translate(${dx}px, ${dy}px)`;
  };
  const dropDrag = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointerId !== event.pointerId) return;
    cancelDrag();
    if (current.active) {
      suppressDoubleClickUntil.current = Date.now() + 600;
      event.preventDefault();
      reorder(current.ids[current.from], current.ids, current.to);
    }
  };
  const blockDragClick = (event: MouseEvent<HTMLDivElement>) => {
    if (suppressClick.current || (event.type === "dblclick" && Date.now() < suppressDoubleClickUntil.current)) { event.preventDefault(); event.stopPropagation(); }
  };
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
    if (event.altKey && ["ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight"].includes(event.key)) {
      event.preventDefault();
      if (index >= 0 && event.target === grid.current?.children[index] && !drag.current) {
        reorder(selected, visible.map((host) => host.id), index + (event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 1), true);
      }
      return;
    }
    if (event.key === "Enter" && index >= 0) { event.preventDefault(); openHost(selected); return; }
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
          <p>{t(favoritesOnly ? "Hosts you starred" : "Manage your saved hosts.")}</p>
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
        <div className={`hosts-${view}`} ref={grid} onKeyDown={moveSelection} onPointerDown={startDrag} onPointerMove={moveDrag} onPointerUp={dropDrag} onPointerCancel={cancelDrag} onLostPointerCapture={cancelDrag} onClickCapture={blockDragClick} onDoubleClickCapture={blockDragClick} onContextMenuCapture={blockDragClick}>
          {visible.map((h) => (
            <HostCard key={h.id} host={h} usage={usageByHost[h.id] ?? UNKNOWN} connectionState={connectionStates.get(h.id)} selected={selected === h.id} busy={busy} onSelect={() => setSelected(h.id)} onConnect={onConnect ? () => openHost(h.id) : undefined} onFavorite={() => toggleFavorite(h)} onEdit={() => edit(h.id)} onMenu={(x, y) => setMenu({ host: h, x, y })} />
          ))}
        </div>
      )}
      {menu && <Menu x={menu.x} y={menu.y} onClose={() => setMenu(null)} items={[
        { label: t("Edit"), icon: "edit", onSelect: () => edit(menu.host.id) },
        { label: t("Duplicate"), icon: "copy", onSelect: () => duplicate(menu.host.id) },
        { label: t("Delete"), icon: "trash", danger: true, onSelect: () => askDelete(menu.host) },
        { label: t(connectionStates.get(menu.host.id) === "connected" ? "New session" : "Connect"), icon: "prompt", disabled: !onConnect, onSelect: () => onConnect?.(menu.host.id) },
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
