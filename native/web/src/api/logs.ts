import { invoke } from "@tauri-apps/api/core";

export interface LogEntry {
  id: string; hostId: string; label: string; address: string; kind: "ssh" | "local";
  startedAt: string; endedAt: string | null; status: "connecting" | "connected" | "closed" | "failed";
  error: string | null; canConnect: boolean;
}
export const listLogs = () => invoke<LogEntry[]>("list_logs");
export const clearLogs = () => invoke<void>("clear_logs");
