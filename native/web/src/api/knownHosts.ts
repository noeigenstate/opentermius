import { invoke } from "@tauri-apps/api/core";

export interface KnownHost { host: string; port: number; fingerprint: string; usedBy: string[] }
export interface ImportSummary { added: number; unchanged: number; conflicts: number; skipped: number }
export const listKnownHosts = () => invoke<KnownHost[]>("list_known_hosts");
export const deleteKnownHost = (host: string, port: number) => invoke<void>("delete_known_host", { host, port });
export const importKnownHosts = (path: string) => invoke<ImportSummary>("import_known_hosts", { path });
