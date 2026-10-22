import { invoke } from "@tauri-apps/api/core";

/** A saved host as the backend exposes it (no credentials). */
export interface Host {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  group: string;
}

/** Usage percentages 0-100; null while unknown. */
export interface Usage {
  cpu: number | null;
  memory: number | null;
  disk: number | null;
}

export const listHosts = () => invoke<Host[]>("list_hosts");
