import { invoke } from "@tauri-apps/api/core";

export interface KeyInfo {
  id: string; label: string; kind: string; fingerprint: string;
  publicKey: string; createdAt: string; usedBy: string[];
}
export type KeyKind = "ed25519" | "rsa3072" | "rsa4096";
export const listKeys = () => invoke<KeyInfo[]>("list_keys");
export const generateKey = (label: string, kind: KeyKind) => invoke<void>("generate_key", { label, kind });
export const keyRequiresPassphrase = (path: string) => invoke<boolean>("key_requires_passphrase", { path });
export const importKey = (path: string, label: string, passphrase: string) => invoke<void>("import_key", { path, label, passphrase: passphrase || null });
export const renameKey = (id: string, label: string) => invoke<void>("rename_key", { id, label });
export const deleteKey = (id: string) => invoke<void>("delete_key", { id });
export const exportPublicKey = (id: string, path: string) => invoke<void>("export_public_key", { id, path });
