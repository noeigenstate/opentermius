import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/api/sessions.ts", import.meta.url), "utf8");
let fixture = 0;
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;

// Isolate the real API module with only its native IPC boundary replaced.
async function createApi() {
  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    export const calls = [];
    export const handlers = new Map();
    export const registrations = new Map();
    let failEvent;
    export function failOn(event) { failEvent = event; }
    export async function invoke(command, args) {
      calls.push({ command, args: structuredClone(args) });
      return command === "list_sessions" ? [] : "created";
    }
    export async function listen(name, callback) {
      registrations.set(name, (registrations.get(name) ?? 0) + 1);
      if (name === failEvent) throw new Error("Fixture listener failure");
      const subscribers = handlers.get(name) ?? new Set();
      subscribers.add(callback);
      handlers.set(name, subscribers);
      return () => { subscribers.delete(callback); if (!subscribers.size) handlers.delete(name); };
    }
    export function emit(name, payload) {
      for (const callback of handlers.get(name) ?? []) callback({ payload });
    }
  `);
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
    .replaceAll('"@tauri-apps/api/core"', JSON.stringify(mockUrl))
    .replaceAll('"@tauri-apps/api/event"', JSON.stringify(mockUrl));
  return { api: await import(moduleUrl(compiled)), mock: await import(mockUrl) };
}

test("commands match the backend signatures and preserve raw Unicode/control input", async () => {
  const { api, mock } = await createApi();
  assert.equal(await api.connectHost("host-fixture"), "created");
  assert.equal(await api.openLocalTerminal(), "created");
  await api.reconnectSession("session-fixture");
  assert.equal(await api.duplicateSession("session-fixture"), "created");
  await api.sessionInput("session-fixture", "\x03中文");
  await api.sessionResize("session-fixture", 132, 40);
  await api.closeSession("session-fixture");
  await api.answerPrompt("prompt-fixture", true, ["fixture-answer", ""]);
  assert.deepEqual(await api.listSessions(), []);
  assert.deepEqual(mock.calls, [
    { command: "connect_host", args: { hostId: "host-fixture" } },
    { command: "open_local_terminal", args: undefined },
    { command: "reconnect_session", args: { sessionId: "session-fixture" } },
    { command: "duplicate_session", args: { sessionId: "session-fixture" } },
    { command: "session_input", args: { sessionId: "session-fixture", data: "\x03中文" } },
    { command: "session_resize", args: { sessionId: "session-fixture", cols: 132, rows: 40 } },
    { command: "close_session", args: { sessionId: "session-fixture" } },
    { command: "answer_prompt", args: { promptId: "prompt-fixture", accepted: true, answers: ["fixture-answer", ""] } },
    { command: "list_sessions", args: undefined },
  ]);
});

test("event listeners are shared and early bytes survive StrictMode effect replay", async () => {
  const { api, mock } = await createApi();
  await Promise.all([api.startSessionEvents(), api.startSessionEvents()]);
  assert.equal(mock.registrations.size, 5);
  assert.deepEqual([...mock.registrations.values()], [1, 1, 1, 1, 1]);
  const bytes = Buffer.from("中文\x1b[31m");
  mock.emit("session://output", { sessionId: "one", data: bytes.toString("base64") });
  const first = [];
  const stopFirst = api.subscribeSession("one", (event) => first.push(event));
  stopFirst();
  const replay = [];
  const stopReplay = api.subscribeSession("one", (event) => replay.push(event));
  assert.deepEqual(first[0].data, new Uint8Array(bytes));
  assert.deepEqual(replay[0].data, new Uint8Array(bytes));
  await Promise.resolve();
  const later = [];
  const stopLater = api.subscribeSession("one", (event) => later.push(event));
  assert.equal(later.length, 0, "retired replay must not duplicate output for later subscribers");
  mock.emit("session://output", { sessionId: "other", data: "eA==" });
  assert.equal(replay.length, 1, "other sessions must not leak into this terminal");
  mock.emit("session://output", { sessionId: "one", data: "eQ==" });
  assert.equal(later[0].data[0], 121);
  stopReplay();
  stopLater();
});

test("status, prompts and stats retain payloads; transport close preserves ended metadata", async () => {
  const { api, mock } = await createApi();
  await api.startSessionEvents();
  const events = [];
  const stop = api.subscribeSessions((event) => events.push(event));
  const status = { sessionId: "one", connected: true, ended: false, error: false, message: "" };
  const prompt = { sessionId: "one", promptId: "prompt", kind: "keyboardInteractive", title: "Fixture", message: "Fixture", prompts: [{ prompt: "Challenge", echo: false }] };
  const stats = { sessionId: "one", hostId: "host-fixture", cpu: 0, memory: null, disk: 100 };
  mock.emit("session://status", status);
  mock.emit("session://prompt", prompt);
  mock.emit("session://stats", stats);
  assert.deepEqual(events, [{ ...status, type: "status" }, { ...prompt, type: "prompt" }, { ...stats, type: "stats" }]);
  mock.emit("session://closed", { sessionId: "one" });
  assert.deepEqual(api.latestSessionStatus("one"), { ...status, connected: false, ended: true });
  api.forgetSession("one");
  assert.equal(api.latestSessionStatus("one"), undefined);
  stop();
  mock.emit("session://closed", { sessionId: "other" });
  assert.equal(events.length, 4);
});

test("partial listener failures roll back and allow a clean retry", async () => {
  const { api, mock } = await createApi();
  mock.failOn("session://stats");
  await assert.rejects(api.startSessionEvents(), /Fixture listener failure/);
  assert.equal(mock.handlers.size, 0);
  mock.failOn(undefined);
  await api.startSessionEvents();
  assert.equal(mock.handlers.size, 5);
  for (const subscribers of mock.handlers.values()) assert.equal(subscribers.size, 1);
});

test("unmounted terminals keep at most one MiB of early output and explicit close releases it", async () => {
  const { api, mock } = await createApi();
  await api.startSessionEvents();
  const large = Buffer.alloc(2 * 1024 * 1024, 120).toString("base64");
  mock.emit("session://output", { sessionId: "one", data: large });
  let bytes = 0;
  const stop = api.subscribeSession("one", (event) => { if (event.type === "output") bytes += event.data.length; });
  assert.equal(bytes, 1024 * 1024);
  stop();
  api.forgetSession("one");
  api.subscribeSession("one", () => assert.fail("forgotten bytes must not replay"))();
});
