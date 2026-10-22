import { invoke } from "@tauri-apps/api/core";

export interface WindTermResult { files: string[]; summary: string | null }
export type RestoreMode = "merge" | "replace";

export const importHostsJson = (path: string) => invoke<string>("import_hosts_json", { path });
export const detectXshellFolder = () => invoke<string | null>("detect_xshell_folder");
export const importXshell = (path?: string) => invoke<string>("import_xshell", { path: path ?? null });
export const importWindterm = (paths?: string[]) => invoke<WindTermResult>("import_windterm", { paths: paths ?? null });
export const exportHosts = (path: string) => invoke<string>("export_hosts", { path });
export const recoveryGenerate = (passphrase: string, includeKeys: boolean) => invoke<string>("recovery_generate", { passphrase: passphrase || null, includeKeys });
export const recoveryRestore = (code: string, passphrase: string, mode: RestoreMode) => invoke<string>("recovery_restore", { code, passphrase: passphrase || null, mode });
