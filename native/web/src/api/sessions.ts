import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Usage } from "./hosts";

/** Session metadata only. Saved credentials are never requested by this module. */
export interface SessionInfo {
  sessionId: string;
  hostId: string;
  title: string;
  local: boolean;
  connected: boolean;
  ended: boolean;
}

export interface SessionStatus {
  hostsChanged?: boolean;
  sessionId: string;
  connected: boolean;
  ended: boolean;
  error: boolean;
  message: string;
}

export interface SessionPrompt {
  promptId: string;
  sessionId: string;
  kind: "username" | "password" | "passphrase" | "hostKey" | "keyboardInteractive";
  title: string;
  message: string;
  fingerprint?: string;
  prompts?: { prompt: string; echo: boolean }[];
}

export interface SessionStats extends Usage {
  sessionId: string;
  hostId: string;
}

export type SessionEvent =
  | { type: "output"; sessionId: string; data: Uint8Array }
  | ({ type: "status" } & SessionStatus)
  | ({ type: "prompt" } & SessionPrompt)
  | ({ type: "stats" } & SessionStats)
  | { type: "closed"; sessionId: string };

type Listener = (event: SessionEvent) => void;
const listeners = new Set<Listener>();
const sessionListeners = new Map<string, Set<Listener>>();
const pendingOutput = new Map<string, { chunks: Uint8Array[]; bytes: number }>();
const statuses = new Map<string, SessionStatus>();
let starting: Promise<void> | undefined;

export function decodeOutput(data: string): Uint8Array {
  return Uint8Array.from(atob(data), (character) => character.charCodeAt(0));
}

function dispatch(event: SessionEvent) {
  if (event.type === "status") statuses.set(event.sessionId, event);
  const subscribers = sessionListeners.get(event.sessionId);
  // Output can arrive before connect_host resolves and before React mounts xterm.
  if (event.type === "output" && !subscribers?.size) {
    const pending = pendingOutput.get(event.sessionId) ?? { chunks: [], bytes: 0 };
    const data = event.data.length > 1024 * 1024 ? event.data.slice(-1024 * 1024) : event.data;
    pending.chunks.push(data);
    pending.bytes += data.length;
    while (pending.bytes > 1024 * 1024 && pending.chunks.length > 1) pending.bytes -= pending.chunks.shift()!.length;
    pendingOutput.set(event.sessionId, pending);
  }
  for (const listener of listeners) listener(event);
  for (const listener of subscribers ?? []) listener(event);
  if (event.type === "closed") {
    const status = statuses.get(event.sessionId);
    statuses.set(event.sessionId, { sessionId: event.sessionId, connected: false, ended: true, error: status?.error ?? false, message: status?.message ?? "" });
  }
}

/** One native listener per event for the lifetime of this webview, including StrictMode. */
export function startSessionEvents(): Promise<void> {
  if (!starting) {
    starting = (async () => {
      const unlisten: UnlistenFn[] = [];
      try {
        unlisten.push(await listen<{ sessionId: string; data: string }>("session://output", ({ payload }) => {
          dispatch({ type: "output", sessionId: payload.sessionId, data: decodeOutput(payload.data) });
        }));
        unlisten.push(await listen<SessionStatus>("session://status", ({ payload }) => dispatch({ ...payload, type: "status" })));
        unlisten.push(await listen<SessionPrompt>("session://prompt", ({ payload }) => dispatch({ ...payload, type: "prompt" })));
        unlisten.push(await listen<SessionStats>("session://stats", ({ payload }) => dispatch({ ...payload, type: "stats" })));
        unlisten.push(await listen<{ sessionId: string }>("session://closed", ({ payload }) => dispatch({ ...payload, type: "closed" })));
      } catch (error) {
        for (const stop of unlisten) stop();
        starting = undefined;
        throw error;
      }
    })();
  }
  return starting;
}

export function subscribeSessions(listener: Listener): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function subscribeSession(sessionId: string, listener: Listener): () => void {
  const subscribers = sessionListeners.get(sessionId) ?? new Set<Listener>();
  subscribers.add(listener);
  sessionListeners.set(sessionId, subscribers);
  const pending = pendingOutput.get(sessionId);
  for (const data of pending?.chunks ?? []) listener({ type: "output", sessionId, data });
  // StrictMode immediately tears down and remounts effects; its second xterm must
  // also receive the replay before we release it at the end of this task.
  queueMicrotask(() => {
    if (sessionListeners.get(sessionId)?.size && pendingOutput.get(sessionId) === pending) pendingOutput.delete(sessionId);
  });
  return () => {
    subscribers.delete(listener);
    if (!subscribers.size) sessionListeners.delete(sessionId);
  };
}

export const latestSessionStatus = (sessionId: string) => statuses.get(sessionId);

export function forgetSession(sessionId: string) {
  pendingOutput.delete(sessionId);
  statuses.delete(sessionId);
}

export async function connectHost(hostId: string): Promise<string> {
  await startSessionEvents();
  return invoke<string>("connect_host", { hostId });
}

export async function openLocalTerminal(): Promise<string> {
  await startSessionEvents();
  return invoke<string>("open_local_terminal");
}

export const reconnectSession = (sessionId: string) => invoke<void>("reconnect_session", { sessionId });
export const duplicateSession = (sessionId: string) => invoke<string>("duplicate_session", { sessionId });
export const sessionResize = (sessionId: string, cols: number, rows: number) => invoke<void>("session_resize", { sessionId, cols, rows });
export const closeSession = (sessionId: string) => invoke<void>("close_session", { sessionId });
export const answerPrompt = (promptId: string, accepted: boolean, answers: string[]) => invoke<void>("answer_prompt", { promptId, accepted, answers });
export const listSessions = () => invoke<SessionInfo[]>("list_sessions");

export const sessionInput = (sessionId: string, data: string) => invoke<void>("session_input", { sessionId, data });
