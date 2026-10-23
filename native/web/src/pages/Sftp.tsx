import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import * as api from "../api/sftp";
import { t } from "../i18n";
import { Icon } from "../icons";

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
  let starting: Promise<void> | undefined;
  let stop: (() => void) | undefined;
  let closed = false;
  const update = (change: Partial<SessionState>) => {
    state = { ...state, ...change };
    observers.forEach((notify) => notify());
  };
  const fail = (error: unknown) => update({ error: String(error), loading: false });
  const finish = (transfer: number, message: string, error: boolean) => update({
    transfers: state.transfers.map((row) => row.transfer === transfer ? { ...row, pending: false, speed: 0, message, error } : row),
    overwrites: state.overwrites.filter((event) => event.transfer !== transfer),
  });
  const offline = () => update({
    connected: false, loading: false, overwrites: [], deletions: [],
    transfers: state.transfers.map((row) => row.pending ? { ...row, pending: false, error: true, speed: 0, message: t("Disconnected") } : row),
  });
  const refresh = async (path = state.path) => {
    update({ loading: true, error: "" });
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
          transfer: (event) => update({ error: event.error ? event.message : "", loading: false }),
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
            if (!event.connected) { offline(); if (event.error) fail(event.message); }
            else { update({ connected: true }); void refresh(); }
          },
          closed: () => { closed = true; offline(); stop?.(); sessions.delete(sessionId); },
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

interface PaneProps {
  side: Side; path: string; entries: api.FileEntry[]; loading: boolean; offline?: boolean;
  showHidden: boolean; selected: string; onSelect: (name: string) => void;
  onNavigate: (path: string) => void; onBack: () => void; backDisabled?: boolean;
  onOpen: (entry: api.FileEntry) => void; onAction: (action: string) => void; onReconnect?: () => void;
  transferDisabled?: boolean;
}
function FilePane(props: PaneProps) {
  const { side, path, entries, selected, onSelect } = props;
  const menu = useRef<HTMLDetailsElement>(null);
  const chooseAction = (operation: string) => { if (menu.current) menu.current.open = false; props.onAction(operation); };
  const [draft, setDraft] = useState(path);
  const [sort, setSort] = useState<{ column: Column; ascending: boolean }>({ column: "name", ascending: true });
  useEffect(() => { setDraft(path); onSelect(""); }, [path, onSelect]);
  const sorted = useMemo(() => entries.filter((entry) => props.showHidden || !entry.name.startsWith(".")).sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    const value = sort.column === "size" ? a.size - b.size
      : sort.column === "modified" ? (seconds(a.modified) || 0) - (seconds(b.modified) || 0)
      : (sort.column === "kind" ? kind(a).localeCompare(kind(b)) : a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
    return (value || a.name.localeCompare(b.name)) * (sort.ascending ? 1 : -1);
  }), [entries, props.showHidden, sort]);
  const title = side === "local" ? t("Local") : t("Remote");
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
      <input aria-label={side === "local" ? t("Local path") : t("Remote path")} value={draft} onChange={(event) => setDraft(event.target.value)} disabled={props.offline} spellCheck={false} />
      <button type="submit" className="btn btn-s" disabled={props.offline || props.loading}>{t("Go")}</button>
      <button type="button" className="icon-button icon-button-boxed" title={t("Refresh")} aria-label={`${title}: ${t("Refresh")}`} disabled={props.offline || props.loading} onClick={() => props.onNavigate(path)}><Icon name="refresh" size={16} /></button>
    </form>
    {props.offline ? <div className="sftp-empty"><span>{t("Not connected — Reconnect")}</span><button className="btn btn-primary" onClick={props.onReconnect}>{t("Reconnect")}</button></div>
      : <div className="sftp-table-wrap" aria-busy={props.loading}>
        <table className="sftp-table"><thead><tr>{([ ["name", "Name"], ["modified", "Modified"], ["size", "Size"], ["kind", "Kind"] ] as [Column, string][]).map(([column, label]) =>
          <th key={column} aria-sort={sort.column === column ? sort.ascending ? "ascending" : "descending" : "none"}><button onClick={() => setSort({ column, ascending: sort.column === column ? !sort.ascending : true })}>{t(label)}{sort.column === column && <Icon name="chevronDown" size={13} />}</button></th>)}</tr></thead>
          <tbody>{sorted.map((entry) => <tr key={entry.name} className={selected === entry.name ? "sftp-selected" : ""} onClick={() => onSelect(entry.name)}>
            <td><button className="sftp-file" onFocus={() => onSelect(entry.name)} onDoubleClick={() => props.onOpen(entry)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); props.onOpen(entry); } }} title={entry.name}>
              <Icon name={entry.isDir ? "folder" : "file"} size={16} /><span>{entry.name}</span>
            </button></td><td>{modified(entry.modified)}</td><td>{entry.isDir ? "—" : size(entry.size)}</td><td>{kind(entry)}</td>
          </tr>)}</tbody></table>
        {sorted.length === 0 && <div className="sftp-empty">{props.loading ? t("Loading…") : t("This folder is empty")}</div>}
      </div>}
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
    session.update({ error: "" });
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
    const pending = current.transfers.filter((item) => item.pending);
    const completed = current.transfers.filter((item) => !item.pending).slice(-Math.max(0, 31 - pending.length));
    session.update({ transfers: [...pending, ...completed, row], error: "" });
    try {
      if (side === "local") await api.upload(sessionId, current.path, local.path, entry.name, id);
      else await api.download(sessionId, current.path, entry.name, local.path, entry.isDir, id);
    } catch (error) { session.finish(id, String(error), true); }
  }

  async function action(side: Side, operation: string) {
    session.update({ error: "" });
    const path = side === "local" ? local.path : remote.path;
    const name = side === "local" ? localSelection : remoteSelection;
    if (operation === "mkdir" || operation === "rename") {
      setValue(operation === "rename" ? name : ""); setDialog({ kind: operation, side, path, name }); return;
    }
    if (!name) { session.fail(t("Select a file or folder first")); return; }
    if (operation === "transfer") { await transfer(side, (side === "local" ? local.entries : remote.entries).find((entry) => entry.name === name)); return; }
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
        transferDisabled={!remote.connected}
        onNavigate={(path) => void loadLocal(path)} onBack={() => local.parent && void loadLocal(local.parent)} backDisabled={!local.parent}
        onAction={(operation) => void action("local", operation)} onOpen={(entry) => entry.isDir ? void loadLocal(entry.path) : void transfer("local", entry)} />
      <FilePane side="remote" path={remote.path} entries={remote.entries} selected={remoteSelection} onSelect={setRemoteSelection} loading={remote.loading} offline={!remote.connected && !remote.loading} showHidden={showHidden}
        onNavigate={(path) => void session.refresh(path)} onBack={() => void session.refresh(parentRemote(remote.path))} backDisabled={remote.path === "/"}
        onAction={(operation) => void action("remote", operation)} onOpen={(entry) => entry.isDir ? void session.refresh(entry.path) : void transfer("remote", entry)}
        onReconnect={() => void run(async () => { await session.start(); await api.reconnect(sessionId); })} />
    </div>
    <section className="sftp-transfers" aria-label={t("Transfers")}><header><strong>{t("Transfers")}</strong><span>{t("8 queued maximum")}</span><span className="sftp-queue-count">{remote.transfers.filter((row) => row.pending).length} / 8</span></header>
      {remote.transfers.length === 0 ? <div className="sftp-no-transfers">{t("No transfers yet")}</div> : <div className="sftp-transfer-list">{[...remote.transfers].sort((a, b) => Number(b.pending) - Number(a.pending) || b.transfer - a.transfer).map((row) => {
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
      })}</div>}
    </section>
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
