import { invoke } from "@tauri-apps/api/core";

/** A saved host as the backend exposes it (no credentials). */
export interface Host {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  group: string;
  tags: string[];
  favorite: boolean;
  lastConnected: string | null;
}

/** Editable fields only. The backend never returns a saved password or encrypted secret. */
export interface HostDraft {
  id: string;
  name: string;
  host: string;
  port: string;
  username: string;
  group: string;
  tags: string;
  authType: "password" | "key";
  privateKeyPath: string;
  keyId: string;
  password?: string;
  hasSecret: boolean;
}

export interface KeychainKey {
  id: string;
  label: string;
  kind: string;
}

export const newHostDraft = (group = ""): HostDraft => ({
  id: "", name: "", host: "", port: "22", username: "", group, tags: "",
  authType: "password", privateKeyPath: "", keyId: "", password: "", hasSecret: false,
});

/** Usage percentages 0-100; null while unknown. */
export interface Usage {
  cpu: number | null;
  memory: number | null;
  disk: number | null;
}

export const listHosts = () => invoke<Host[]>("list_hosts");
export const getHost = (id: string) => invoke<HostDraft>("get_host", { id });
export const saveHost = (draft: HostDraft) => invoke<string>("save_host", { draft });
export const deleteHost = (id: string) => invoke<void>("delete_host", { id });
export const setHostFavorite = (id: string, favorite: boolean) => invoke<void>("set_host_favorite", { id, favorite });
export const duplicateHost = (id: string) => invoke<string>("duplicate_host", { id });
export const renameGroup = (old: string, next: string) => invoke<void>("rename_group", { old, new: next });
export const listKeychainKeys = () => invoke<KeychainKey[]>("list_keychain_keys");
export const parseQuickConnect = (text: string) => invoke<HostDraft>("parse_quick_connect", { text });
