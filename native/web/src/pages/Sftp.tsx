import { Fragment, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import * as api from "../api/sftp";
import { t } from "../i18n";
import { Icon, type IconName } from "../icons";
import { Menu, type MenuItem } from "../components/Menu";

type Side = "local" | "remote";
type Column = "name" | "modified" | "size" | "kind";
interface Transfer {
  transfer: number; name: string; direction: string; transferred: number; total: number;
  speed: number; message: string; pending: boolean; error: boolean; cancelling?: boolean;
}
interface SessionState {
  path: string; entries: api.FileEntry[]; connected: boolean; loading: boolean; error: string;
  transfers: Transfer[]; overwrites: api.OverwriteEvent[]; deletions: api.DeleteEvent[]; revision: number;
  localPath: string; showHidden: boolean;
}

// Keep listening while another session/tab is visible, so completions and confirmations
// are not lost on unmount. The session://closed event releases the listener set.
const sessions = new Map<string, ReturnType<typeof createSession>>();
let nextTransfer = Date.now();
function createSession(sessionId: string) {
  let state: SessionState = { path: ".", entries: [], connected: false, loading: true, error: "", transfers: [], overwrites: [], deletions: [], revision: 0, localPath: "", showHidden: false };
  const observers = new Set<() => void>();
  const removals = new Map<number, ReturnType<typeof setTimeout>>();
  let starting: Promise<void> | undefined;
  let stop: (() => void) | undefined;
  let closed = false;
  const update = (change: Partial<SessionState>) => {
    state = { ...state, ...change };
    observers.forEach((notify) => notify());
  };
  const fail = (error: unknown) => update({ error: String(error), loading: false });
  const finish = (transfer: number, message: string, error: boolean) => {
    const row = state.transfers.find((row) => row.transfer === transfer);
    if (!row) return;
    update({
      transfers: state.transfers.map((row) => row.transfer === transfer ? { ...row, pending: false, speed: 0, message, error } : row),
      overwrites: state.overwrites.filter((event) => event.transfer !== transfer),
      ...(error ? { error: `${row.direction} · ${row.name}: ${message}` } : {}),
    });
    clearTimeout(removals.get(transfer));
    if (closed) return;
    removals.set(transfer, setTimeout(() => {
      removals.delete(transfer);
      update({ transfers: state.transfers.filter((row) => row.transfer !== transfer) });
    }, 1500));
  };
  const offline = () => {
    update({ connected: false, loading: false, overwrites: [], deletions: [] });
    state.transfers.filter((row) => row.pending).forEach((row) => finish(row.transfer, t("Disconnected"), true));
  };
  const refresh = async (path = state.path) => {
    update({ loading: true });
    try { await api.remoteList(sessionId, path); }
    catch (error) { fail(error); }
  };
  return {
    snapshot: () => state,
    subscribe: (notify: () => void) => { observers.add(notify); return () => { observers.delete(notify); }; },
    update, fail, finish, refresh,
    start: () => starting ??= (async () => {
      try {
        stop = await api.listenSftp(sessionId, {
          files: (event) => update({ path: event.path, entries: event.entries, connected: true, loading: false }),
          transfer: (event) => update({ error: event.error ? event.message : state.error, loading: false }),
          progress: (event) => {
            if (!state.transfers.some((row) => row.transfer === event.transfer)) return;
            update({ transfers: state.transfers.map((row) => row.transfer === event.transfer && row.pending ? { ...row, ...event, message: "" } : row) });
          },
          finished: (event) => {
            finish(event.transfer, event.message, event.error);
            update({ revision: state.revision + 1 });
            if (state.connected) void refresh();
          },
          overwrite: (event) => {
            // A cancelled/finished upload must never resurrect an overwrite dialog.
            const row = state.transfers.find((item) => item.transfer === event.transfer);
            if (row?.cancelling) { finish(event.transfer, t("Cancelled"), false); return; }
            if (event.transfer !== 0 && !row?.pending) return;
            if (!state.overwrites.some((item) => item.transfer === event.transfer && item.local === event.local)) {
              update({ overwrites: [...state.overwrites, event] });
            }
          },
          delete: (event) => update({ deletions: [...state.deletions, event] }),
          status: (event) => {
            if (!event.connected) { offline(); if (event.error) fail(event.message); } // the real reason replaces the per-row "Disconnected" messages
            else { update({ connected: true }); void refresh(); }
          },
          closed: () => {
            closed = true; offline(); stop?.();
            removals.forEach(clearTimeout); removals.clear();
            sessions.delete(sessionId);
          },
        });
        if (closed) stop();
        else await refresh();
      } catch (error) { starting = undefined; fail(error); }
    })(),
  };
}

function getSession(sessionId: string) {
  let session = sessions.get(sessionId);
  if (!session) { session = createSession(sessionId); sessions.set(sessionId, session); }
  return session;
}

function size(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const unit = bytes < 1024 ? 0 : Math.min(4, Math.floor(Math.log(bytes) / Math.log(1024)));
  return `${(bytes / 1024 ** unit).toLocaleString(undefined, { maximumFractionDigits: unit ? 1 : 0 })} ${["B", "KB", "MB", "GB", "TB"][unit]}`;
}
function kind(entry: api.FileEntry) {
  const extension = entry.name.includes(".") ? entry.name.split(".").at(-1) : "";
  return entry.isDir ? t("Folder") : extension ? t("{extension} file", { extension: extension.toUpperCase() }) : t("File");
}
function seconds(value: number | string) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : Date.parse(String(value)) / 1000;
}
function modified(value: number | string) {
  const timestamp = seconds(value);
  return timestamp > 0 ? new Date(timestamp * 1000).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" }) : "—";
}
function parentRemote(path: string) {
  if (path === "/") return "/";
  if (path === ".") return "..";
  return `${path.replace(/\/$/, "")}/..`;
}

// Explorer-style glyphs: folders and plain files stay neutral, git and document files get a tone (their shapes differ too, so colour is never the only cue).
function fileIcon(entry: api.FileEntry): { icon: IconName; tone?: "git" | "doc" } {
  if (entry.isDir) return { icon: "folder" };
  if (/^\.git(?:ignore|attributes|modules|keep)$/i.test(entry.name)) return { icon: "gitBranch", tone: "git" };
  if (/\.(?:md|markdown|txt|log|rst)$|^(?:readme|license|changelog)(?:\.|$)/i.test(entry.name)) return { icon: "fileText", tone: "doc" };
  return { icon: "file" };
}

const placeIcons: Record<api.LocalPlace["kind"], IconName> = { home: "home", desktop: "desktop", downloads: "download", documents: "fileText", drive: "drive" };
// Explorer-style drive names ("Data (D:)", "Local disk (C:)"); systems without drive letters show the mount name or root alone.
function placeLabel({ kind, path, name }: api.LocalPlace) {
  const known = { home: t("Home"), desktop: t("Desktop"), downloads: t("Downloads"), documents: t("Documents") };
  if (kind !== "drive") return known[kind];
  const letter = /^[A-Za-z]:/.exec(path)?.[0];
  return letter ? `${name || t("Local disk")} (${letter})` : name || path;
}
const trimmed = (path: string) => path.replace(/[\\/]+$/, "").toLowerCase();

/** The local path input as an editable combobox: its list holds the user folders and drives, and picking one navigates like typing the path and pressing Go. */
function PlacesField({ label, value, current, disabled, onChange, onPick, onError }: {
  label: string; value: string; current: string; disabled?: boolean;
  onChange: (value: string) => void; onPick: (path: string) => void; onError?: (error: unknown) => void;
}) {
  const id = useId();
  const field = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [places, setPlaces] = useState<api.LocalPlace[] | null>(null); // null while the list is closed
  const [active, setActive] = useState(0);
  const [keys, setKeys] = useState(false); // keyboard highlights get the focus ring, pointer ones only the hover fill
  const open = places !== null;
  const isCurrent = (place: api.LocalPlace) => current !== "" && trimmed(place.path) === trimmed(current);
  const firstDrive = places?.findIndex((place) => place.kind === "drive") ?? -1;

  async function show(byKeys: boolean) {
    try {
      const found = await api.localPlaces(); // drives come and go, so every opening asks again
      if (found.length === 0) return;
      setPlaces(found); setActive(Math.max(0, found.findIndex(isCurrent))); setKeys(byKeys);
    } catch (error) { onError?.(error); }
  }
  const pick = (place: api.LocalPlace) => { setPlaces(null); onChange(place.path); onPick(place.path); };
  function onKeyDown(event: { key: string; preventDefault: () => void }) {
    if (!places) {
      if (event.key === "ArrowDown") { event.preventDefault(); void show(true); }
      return;
    }
    const last = places.length - 1;
    const next = event.key === "ArrowDown" ? (active + 1) % places.length : event.key === "ArrowUp" ? (active + last) % places.length : event.key === "Home" ? 0 : event.key === "End" ? last : -1;
    if (next >= 0) { event.preventDefault(); setActive(next); setKeys(true); }
    else if (event.key === "Enter") { event.preventDefault(); pick(places[active]); }
    else if (event.key === "Escape") { event.preventDefault(); setPlaces(null); }
    else if (event.key === "Tab") setPlaces(null);
  }

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => { if (!field.current?.contains(event.target as Node)) setPlaces(null); };
    const dismiss = () => setPlaces(null);
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("scroll", outside, true); // scrolling the list itself keeps it open
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("scroll", outside, true);
    };
  }, [open]);
  useEffect(() => { field.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" }); }, [open, active]);

  return <div className="sftp-path-field" ref={field}>
    <input ref={input} role="combobox" aria-label={label} aria-expanded={open} aria-haspopup="listbox" aria-controls={`${id}-list`} aria-autocomplete="none" aria-activedescendant={open ? `${id}-${active}` : undefined}
      value={value} disabled={disabled} spellCheck={false} onChange={(event) => { setPlaces(null); onChange(event.target.value); }} onKeyDown={onKeyDown} />
    <button type="button" tabIndex={-1} className="icon-button sftp-path-toggle" title={t("Places")} aria-label={t("Places")} aria-expanded={open} aria-controls={`${id}-list`} disabled={disabled}
      onMouseDown={(event) => event.preventDefault()} onClick={() => { input.current?.focus(); if (open) setPlaces(null); else void show(false); }}><Icon name="chevronDown" size={14} /></button>
    {places && <div className="menu sftp-places" id={`${id}-list`} role="listbox" aria-label={t("Places")} data-keys={keys || undefined} onMouseDown={(event) => event.preventDefault()}>
      {places.map((place, index) => <Fragment key={`${place.kind}:${place.path}`}>
        {index > 0 && index === firstDrive && <hr aria-hidden />}
        <button type="button" role="option" id={`${id}-${index}`} tabIndex={-1} aria-selected={index === active} aria-current={isCurrent(place) || undefined}
          onPointerEnter={() => { setActive(index); setKeys(false); }} onClick={() => pick(place)}>
          <Icon name={placeIcons[place.kind]} size={16} />
          <span>{placeLabel(place)}</span>
          {place.kind !== "drive" && <span className="place-path">{place.path}</span>}
          <span className="place-mark">{isCurrent(place) && <Icon name="check" size={14} />}</span>
        </button>
      </Fragment>)}
    </div>}
  </div>;
}

interface PaneProps {
  side: Side; path: string; entries: api.FileEntry[]; loading: boolean; offline?: boolean;
  showHidden: boolean; selected: string; onSelect: (name: string) => void;
  onNavigate: (path: string) => void; onBack: () => void; backDisabled?: boolean;
  onOpen: (entry: api.FileEntry) => void; onAction: (action: string, name?: string) => void; onReconnect?: () => void;
  transferDisabled?: boolean; onError?: (error: unknown) => void;
}
function FilePane(props: PaneProps) {
  const { side, path, entries, selected, onSelect } = props;
  const menu = useRef<HTMLDetailsElement>(null);
  const chooseAction = (operation: string) => { if (menu.current) menu.current.open = false; props.onAction(operation); };
  const [draft, setDraft] = useState(path);
  const [sort, setSort] = useState<{ column: Column; ascending: boolean }>({ column: "name", ascending: true });
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; entry?: api.FileEntry } | null>(null);
  useEffect(() => { setDraft(path); onSelect(""); }, [path, onSelect]);
  const sorted = useMemo(() => entries.filter((entry) => props.showHidden || !entry.name.startsWith(".")).sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    const value = sort.column === "size" ? a.size - b.size
      : sort.column === "modified" ? (seconds(a.modified) || 0) - (seconds(b.modified) || 0)
      : (sort.column === "kind" ? kind(a).localeCompare(kind(b)) : a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
    return (value || a.name.localeCompare(b.name)) * (sort.ascending ? 1 : -1);
  }), [entries, props.showHidden, sort]);
  const title = side === "local" ? t("Local") : t("Remote");
  const entry = contextMenu?.entry;
  const items: MenuItem[] = entry ? [
    ...(entry.isDir ? [{ label: t("Open"), icon: "folder" as const, disabled: props.offline, onSelect: () => props.onOpen(entry) }] : []),
    { label: side === "local" ? t("Upload") : t("Download"), icon: side === "local" ? "arrowUp" : "download", disabled: props.offline || props.transferDisabled, onSelect: () => props.onAction("transfer", entry.name) },
    { label: t("Copy name"), icon: "copy", onSelect: () => props.onAction("copyName", entry.name) },
    { label: t("Copy path"), icon: "copy", onSelect: () => props.onAction("copyPath", entry.name) },
    { label: t("Rename"), icon: "edit", disabled: props.offline, onSelect: () => props.onAction("rename", entry.name) },
    { label: t("Delete"), icon: "trash", danger: true, disabled: props.offline, onSelect: () => props.onAction("delete", entry.name) },
  ] : [
    { label: t("New folder"), icon: "plus", disabled: props.offline || !path, onSelect: () => props.onAction("mkdir") },
    { label: t("Refresh"), icon: "refresh", disabled: props.offline || props.loading, onSelect: () => props.onNavigate(path) },
  ];
  return <section className="sftp-pane" aria-label={title}>
    <header className="sftp-pane-head"><strong>{title}</strong>
      <span className={`dot${side === "local" || !props.offline ? " dot-online" : ""}`} />
      <details className="sftp-actions" ref={menu}><summary className="btn btn-ghost btn-s">{t("Actions")}<Icon name="chevronDown" size={14} /></summary><div className="menu sftp-menu">
        <button disabled={props.offline || !path} onClick={() => chooseAction("mkdir")}>{t("New folder")}</button>
        <button disabled={!selected || props.offline} onClick={() => chooseAction("rename")}>{t("Rename")}</button>
        <button disabled={!selected || props.offline || props.transferDisabled} onClick={() => chooseAction("transfer")}>{side === "local" ? t("Upload") : t("Download")}</button>
        <button className="menu-danger" disabled={!selected || props.offline} onClick={() => chooseAction("delete")}>{t("Delete")}</button>
      </div></details>
    </header>
    <form className="sftp-path" onSubmit={(event) => { event.preventDefault(); props.onNavigate(draft); }}>
      <button type="button" className="icon-button icon-button-boxed" title={t("Back")} aria-label={`${title}: ${t("Back")}`} disabled={props.backDisabled || props.offline || props.loading} onClick={props.onBack}><Icon name="arrowUp" size={16} /></button>
      {side === "local" ? <PlacesField label={t("Local path")} value={draft} current={path} disabled={props.offline} onChange={setDraft} onPick={props.onNavigate} onError={props.onError} />
        : <input aria-label={t("Remote path")} value={draft} onChange={(event) => setDraft(event.target.value)} disabled={props.offline} spellCheck={false} />}
      <button type="submit" className="btn btn-s" disabled={props.offline || props.loading}>{t("Go")}</button>
      <button type="button" className="icon-button icon-button-boxed" title={t("Refresh")} aria-label={`${title}: ${t("Refresh")}`} disabled={props.offline || props.loading} onClick={() => props.onNavigate(path)}><Icon name="refresh" size={16} /></button>
    </form>
    <div className="sftp-table-wrap" aria-busy={props.loading} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ x: event.clientX, y: event.clientY }); }}>
      {props.offline ? <div className="sftp-empty"><span>{t("Not connected — Reconnect")}</span><button className="btn btn-primary" onClick={props.onReconnect}>{t("Reconnect")}</button></div>
      : <>
        <table className="sftp-table"><thead><tr>{([ ["name", "Name"], ["modified", "Modified"], ["size", "Size"], ["kind", "Kind"] ] as [Column, string][]).map(([column, label]) =>
          <th key={column} aria-sort={sort.column === column ? sort.ascending ? "ascending" : "descending" : "none"}><button onClick={() => setSort({ column, ascending: sort.column === column ? !sort.ascending : true })}>{t(label)}{sort.column === column && <Icon name="chevronDown" size={13} />}</button></th>)}</tr></thead>
          <tbody>{sorted.map((entry) => { const { icon, tone } = fileIcon(entry); return <tr key={entry.name} className={selected === entry.name ? "sftp-selected" : ""} onClick={() => onSelect(entry.name)}
            onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); onSelect(entry.name); setContextMenu({ x: event.clientX, y: event.clientY, entry }); }}>
            <td><button className="sftp-file" data-tone={tone} onFocus={() => onSelect(entry.name)} onDoubleClick={() => props.onOpen(entry)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); props.onOpen(entry); } }} title={entry.name}>
              <Icon name={icon} size={16} /><span>{entry.name}</span>
            </button></td><td>{modified(entry.modified)}</td><td>{entry.isDir ? "—" : size(entry.size)}</td><td>{kind(entry)}</td>
          </tr>; })}</tbody></table>
        {sorted.length === 0 && <div className="sftp-empty">{props.loading ? t("Loading…") : t("This folder is empty")}</div>}
      </>}
    </div>
    {contextMenu && <Menu x={contextMenu.x} y={contextMenu.y} items={items} onClose={() => setContextMenu(null)} />}
  </section>;
}

type Dialog = { kind: "mkdir" | "rename"; side: Side; path: string; name: string }
  | { kind: "localDelete"; path: string; name: string; summary: api.DeletionSummary };

/** Session owners can mount this view directly beside their terminal. */
export function SftpView({ sessionId }: { sessionId: string }) {
  return <SftpWorkspace key={sessionId} sessionId={sessionId} />;
}

function SftpWorkspace({ sessionId }: { sessionId: string }) {
  const session = useMemo(() => getSession(sessionId), [sessionId]);
  const remote = useSyncExternalStore(session.subscribe, session.snapshot);
  const [local, setLocal] = useState<api.LocalListing>({ path: "", parent: null, entries: [] });
  const [localBusy, setLocalBusy] = useState(true);
  const showHidden = remote.showHidden;
  const [localSelection, setLocalSelection] = useState("");
  const [remoteSelection, setRemoteSelection] = useState("");
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [value, setValue] = useState("");
  const [dialogBusy, setDialogBusy] = useState(false);
  const localRequest = useRef(0);
  const localPath = useRef(remote.localPath);
  const hidden = useRef(showHidden);
  hidden.current = showHidden;
  const alive = useRef(true);

  async function loadLocal(path: string) {
    const generation = ++localRequest.current;
    setLocalBusy(true);
    try {
      const result = await api.localList(path || await api.localHome(), hidden.current);
      if (alive.current && generation === localRequest.current) {
        localPath.current = result.path; setLocal(result); setLocalSelection(""); session.update({ localPath: result.path });
      }
    } catch (error) { if (alive.current && generation === localRequest.current) session.fail(error); }
    finally { if (alive.current && generation === localRequest.current) setLocalBusy(false); }
  }

  useEffect(() => {
    alive.current = true;
    void session.start();
    return () => { alive.current = false; localRequest.current++; };
  }, [session]);
  useEffect(() => { void loadLocal(localPath.current); }, [showHidden, remote.revision]); // Reload after completed downloads, including while this tab was hidden.

  async function run(work: () => Promise<unknown>) {
    try { await work(); } catch (error) { session.fail(error); }
  }

  async function transfer(side: Side, entry?: api.FileEntry) {
    if (!entry || !remote.connected || !local.path) return;
    const current = session.snapshot();
    if (current.transfers.filter((row) => row.pending).length >= 8) {
      session.fail(t("Transfer queue is full (up to 8 tasks). Wait or cancel a task")); return;
    }
    const id = ++nextTransfer;
    const row: Transfer = { transfer: id, name: entry.name, direction: side === "local" ? t("Upload") : t("Download"), transferred: 0, total: entry.size, speed: 0, message: t("Queued"), pending: true, error: false };
    session.update({ transfers: [...current.transfers, row] }); // queue order: the panel shows the first three, finished rows keep their place until removed
    try {
      if (side === "local") await api.upload(sessionId, current.path, local.path, entry.name, id);
      else await api.download(sessionId, current.path, entry.name, local.path, entry.isDir, id);
    } catch (error) { session.finish(id, String(error), true); }
  }

  async function action(side: Side, operation: string, selectedName?: string) {
    const path = side === "local" ? local.path : remote.path;
    const name = selectedName ?? (side === "local" ? localSelection : remoteSelection);
    if (operation === "mkdir" || operation === "rename") {
      setValue(operation === "rename" ? name : ""); setDialog({ kind: operation, side, path, name }); return;
    }
    if (!name) { session.fail(t("Select a file or folder first")); return; }
    const entry = (side === "local" ? local.entries : remote.entries).find((entry) => entry.name === name);
    if (operation === "transfer") { await transfer(side, entry); return; }
    if ((operation === "copyName" || operation === "copyPath") && entry) {
      const fullPath = side === "local" ? entry.path : path ? `${path.replace(/\/+$/, "")}/${name}` : name;
      try { await navigator.clipboard.writeText(operation === "copyName" ? name : fullPath); }
      catch { session.fail(t("Cannot access clipboard")); }
      return;
    }
    if (operation === "delete") await run(async () => {
      if (side === "remote") await api.remoteDelete(sessionId, path, name);
      else setDialog({ kind: "localDelete", path, name, summary: await api.localDelete(path, name) });
    });
  }

  const overwrite = remote.overwrites[0];
  const deletion = remote.deletions[0];
  const activeDialog = dialog || overwrite || deletion;
  const naming = dialog?.kind === "mkdir" || dialog?.kind === "rename";
  const dialogTitle = naming ? dialog.kind === "mkdir" ? t("New folder") : t("Rename") : dialog || deletion && !overwrite ? t("Confirm deletion") : t("Overwrite remote file");
  const summary = dialog?.kind === "localDelete" ? dialog.summary : deletion;
  const deletionName = dialog?.kind === "localDelete" ? dialog.name : deletion?.name;

  function dismissDialog() {
    if (dialog) setDialog(null);
    else if (overwrite) {
      session.finish(overwrite.transfer, t("Cancelled"), false);
      session.update({ overwrites: session.snapshot().overwrites.filter((event) => event !== overwrite) });
    } else session.update({ deletions: session.snapshot().deletions.slice(1) });
  }
  async function acceptDialog() {
    setDialogBusy(true);
    try {
      if (dialog) {
        if (dialog.kind === "localDelete") await api.localDelete(dialog.path, dialog.name, true);
        else if (dialog.side === "local") {
          if (dialog.kind === "mkdir") await api.localMkdir(dialog.path, value);
          else await api.localRename(dialog.path, dialog.name, value);
        } else if (dialog.kind === "mkdir") await api.remoteMkdir(sessionId, dialog.path, value);
        else await api.remoteRename(sessionId, dialog.path, dialog.name, value);
        setDialog(null);
        await loadLocal(localPath.current);
      } else if (overwrite) {
        await api.confirmOverwrite(overwrite);
        session.update({ overwrites: session.snapshot().overwrites.filter((event) => event !== overwrite) });
      } else if (deletion) {
        await api.confirmDelete(deletion);
        session.update({ deletions: session.snapshot().deletions.filter((event) => event !== deletion) });
      }
    } catch (error) { session.fail(error); }
    finally { setDialogBusy(false); }
  }

  const destructive = !naming && (Boolean(dialog) || !overwrite);
  return <section className="sftp-page" aria-label={t("SFTP")}>
    <div className="sftp-toolbar"><span className="sftp-hint">{t("Double-click a folder to open it, or a file to transfer it")}</span>
      <label className="sftp-hidden-toggle"><span>{t("Show hidden files")}</span><input type="checkbox" role="switch" checked={showHidden} onChange={(event) => session.update({ showHidden: event.target.checked })} /></label>
    </div>
    {remote.error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{remote.error}</span><button className="icon-button icon-button-sm" aria-label={t("Close")} onClick={() => session.update({ error: "" })}><Icon name="close" size={14} /></button></div>}
    <div className="sftp-panes">
      <FilePane side="local" path={local.path} entries={local.entries} selected={localSelection} onSelect={setLocalSelection} loading={localBusy} showHidden={showHidden}
        transferDisabled={!remote.connected} onError={(error) => session.update({ error: String(error) })}
        onNavigate={(path) => void loadLocal(path)} onBack={() => local.parent && void loadLocal(local.parent)} backDisabled={!local.parent}
        onAction={(operation, name) => void action("local", operation, name)} onOpen={(entry) => entry.isDir ? void loadLocal(entry.path) : void transfer("local", entry)} />
      <FilePane side="remote" path={remote.path} entries={remote.entries} selected={remoteSelection} onSelect={setRemoteSelection} loading={remote.loading} offline={!remote.connected && !remote.loading} showHidden={showHidden}
        onNavigate={(path) => void session.refresh(path)} onBack={() => void session.refresh(parentRemote(remote.path))} backDisabled={remote.path === "/"}
        onAction={(operation, name) => void action("remote", operation, name)} onOpen={(entry) => entry.isDir ? void session.refresh(entry.path) : void transfer("remote", entry)}
        onReconnect={() => void run(async () => { await session.start(); await api.reconnect(sessionId); })} />
    </div>
    {remote.transfers.length > 0 && <section className="sftp-transfers" aria-label={t("Transfers")}><header><strong>{t("Transfers")}</strong><span>{t("8 queued maximum")}</span><span className="sftp-queue-count">{remote.transfers.filter((row) => row.pending).length} / 8</span>{remote.transfers.length > 3 && <span>{t("{count} more", { count: remote.transfers.length - 3 })}</span>}</header>
      <div className="sftp-transfer-list">{remote.transfers.slice(0, 3).map((row) => {
        const percent = row.total > 0 ? Math.min(100, Math.round(row.transferred / row.total * 100)) : !row.pending && !row.error ? 100 : 0;
        return <div className={`sftp-transfer${row.error ? " sftp-transfer-error" : ""}`} key={row.transfer}><div className="sftp-transfer-detail"><strong>{row.direction} · {row.name}</strong><span>{row.error && <Icon name="alert" size={14} />}{row.message || `${size(row.transferred)} / ${size(row.total)} · ${size(row.speed)}/s`}</span>
          <progress aria-label={`${row.direction} · ${row.name}`} max={100} value={percent} /></div>
          <span className="sftp-percent">{percent}%</span><button className="btn btn-ghost btn-s" disabled={!row.pending || row.cancelling} onClick={() => void run(async () => {
            await api.cancelTransfer(sessionId, row.transfer);
            const snapshot = session.snapshot();
            // Overwrite requests are paused outside the worker: cancellation has no
            // completion event until resumed, so finish that row locally.
            if (snapshot.overwrites.some((event) => event.transfer === row.transfer)) session.finish(row.transfer, t("Cancelled"), false);
            else session.update({ transfers: snapshot.transfers.map((item) => item.transfer === row.transfer && item.pending ? { ...item, cancelling: true, message: t("Cancelling…") } : item) });
          })}>{row.cancelling && row.pending ? t("Cancelling…") : t("Cancel")}</button></div>;
      })}</div>
    </section>}
    {activeDialog && <dialog className="dialog" aria-labelledby="sftp-dialog-title" ref={(node) => { if (node && !node.open) node.showModal(); }} onCancel={(event) => { event.preventDefault(); if (!dialogBusy) dismissDialog(); }}>
      <form onSubmit={(event) => { event.preventDefault(); void acceptDialog(); }}>
        <h2 id="sftp-dialog-title">{dialogTitle}</h2>
        {naming ? <label className="field">{dialog.kind === "mkdir" ? t("Folder name") : t("New name")}<input autoFocus value={value} onChange={(event) => setValue(event.target.value)} required disabled={dialogBusy} /></label>
          : <p>{!dialog && overwrite ? t(overwrite.folder ? "A folder with this name already exists. Merge and overwrite matching files?" : "A file with this name already exists. Overwrite it?")
            : t("Delete {name}? {files} files, {dirs} subfolders, {size}. This cannot be undone.", { name: deletionName || "", files: summary?.files ?? 0, dirs: summary?.dirs ?? 0, size: size(summary?.bytes ?? 0) })}</p>}
        {remote.error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{remote.error}</span></div>}
        <div className="dialog-actions"><button type="button" className="btn" autoFocus={!naming} disabled={dialogBusy} onClick={dismissDialog}>{t("Cancel")}</button><button className={`btn ${destructive ? "btn-danger" : "btn-primary"}`} type="submit" disabled={dialogBusy || (naming && !value.trim())}>{naming ? dialog.kind === "mkdir" ? t("Create") : t("Save") : !dialog && overwrite ? t("Overwrite") : t("Delete")}</button></div>
      </form>
    </dialog>}
  </section>;
}
