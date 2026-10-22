import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface FileEntry {
  name: string;
  path: string;
  size: number;
  isDir: boolean;
  modified: string | number;
  permissions?: number | null;
}
export interface LocalListing { path: string; parent: string | null; entries: FileEntry[] }
export interface DeletionSummary { deleted?: boolean; files: number; dirs: number; bytes: number }
// SFTP payloads are the camelCase ssh::Event variants (id is the session id).
export interface FilesEvent { id: string; path: string; entries: FileEntry[] }
export interface TransferEvent { id: string; message: string; error: boolean }
export interface ProgressEvent {
  id: string; transfer: number; name: string; direction: string;
  transferred: number; total: number; speed: number;
}
export interface FinishedEvent extends TransferEvent { transfer: number }
export interface OverwriteEvent {
  id: string; localRoot: string; directory: string; local: string; transfer: number; folder: boolean;
}
export interface DeleteEvent extends DeletionSummary { id: string; directory: string; name: string }
export interface StatusEvent { sessionId: string; connected: boolean; ended: boolean; error: boolean; message: string }

export const localHome = () => invoke<string>("local_home");
export const localList = (path: string, showHidden: boolean) => invoke<LocalListing>("local_list", { path, showHidden });
export const localMkdir = (path: string, name: string) => invoke<void>("local_mkdir", { path, name });
export const localRename = (path: string, from: string, to: string) => invoke<void>("local_rename", { path, from, to });
export const localDelete = (path: string, name: string, confirmed = false) => invoke<DeletionSummary>("local_delete", { path, name, confirmed });
export const remoteList = (sessionId: string, path: string) => invoke<void>("sftp_list", { sessionId, path });
export const upload = (sessionId: string, directory: string, localRoot: string, name: string, transfer: number) =>
  invoke<void>("sftp_upload", { sessionId, directory, localRoot, name, transfer });
export const download = (sessionId: string, directory: string, name: string, localRoot: string, folder: boolean, transfer: number) =>
  invoke<void>("sftp_download", { sessionId, directory, name, localRoot, folder, transfer });
export const remoteMkdir = (sessionId: string, directory: string, name: string) => invoke<void>("sftp_mkdir", { sessionId, directory, name });
export const remoteRename = (sessionId: string, directory: string, from: string, to: string) => invoke<void>("sftp_rename", { sessionId, directory, from, to });
export const remoteDelete = (sessionId: string, directory: string, name: string) => invoke<void>("sftp_delete", { sessionId, directory, name });
export const confirmDelete = ({ id: sessionId, directory, name }: DeleteEvent) => invoke<void>("sftp_confirm_delete", { sessionId, directory, name });
export const confirmOverwrite = ({ id: sessionId, ...event }: OverwriteEvent) => invoke<void>("sftp_confirm_overwrite", { sessionId, ...event });
export const cancelTransfer = (sessionId: string, transfer: number) => invoke<void>("sftp_cancel_transfer", { sessionId, transfer });
export const reconnect = (sessionId: string) => invoke<void>("session_reconnect", { sessionId });

export interface SftpHandlers {
  files: (event: FilesEvent) => void;
  transfer: (event: TransferEvent) => void;
  progress: (event: ProgressEvent) => void;
  finished: (event: FinishedEvent) => void;
  overwrite: (event: OverwriteEvent) => void;
  delete: (event: DeleteEvent) => void;
  status: (event: StatusEvent) => void;
  closed: () => void;
}

/** Attach every listener before requesting a listing; clean up partial setup failures. */
export async function listenSftp(sessionId: string, handlers: SftpHandlers): Promise<UnlistenFn> {
  const subscriptions: UnlistenFn[] = [];
  const add = async <T,>(name: string, handler: (event: T) => void, matches: (event: T) => boolean) => {
    subscriptions.push(await listen<T>(name, ({ payload }) => { if (matches(payload)) handler(payload); }));
  };
  const mine = (event: { id: string }) => event.id === sessionId;
  const session = (event: { sessionId: string }) => event.sessionId === sessionId;
  try {
    await add("sftp://files", handlers.files, mine);
    await add("sftp://transfer", handlers.transfer, mine);
    await add("sftp://progress", handlers.progress, mine);
    await add("sftp://finished", handlers.finished, mine);
    await add("sftp://overwrite", handlers.overwrite, mine);
    await add("sftp://delete", handlers.delete, mine);
    await add("session://status", handlers.status, session);
    await add<{ sessionId: string }>("session://closed", handlers.closed, session);
    return () => subscriptions.forEach((stop) => stop());
  } catch (error) {
    subscriptions.forEach((stop) => stop());
    throw error;
  }
}
